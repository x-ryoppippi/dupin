import { zValidator } from '@hono/zod-validator';
import { Hono } from 'hono';
import type { FromSchema } from 'json-schema-to-ts';
import type { CloudEnv } from 'void';
import { defineHandler } from 'void';
import * as z from 'zod';
import type { JSONSchema } from 'zod/v4/core';

import { pingJsonSchema } from './v1/ping-schema.gen.ts';

const MAX_BODY_BYTES = 1024;

export const headerSchema = z.object({
	'content-type': z.string().startsWith('application/json'),
	'content-length': z.coerce.number().int().nonnegative().max(MAX_BODY_BYTES),
});

export type Ping = FromSchema<typeof pingJsonSchema>;

type Mutable<T> = T extends readonly (infer Item)[]
	? Mutable<Item>[]
	: T extends object
		? { -readonly [Key in keyof T]: Mutable<T[Key]> }
		: T;

// Zod's JSON Schema input requires mutable arrays, while the generated `as const` schema is readonly. `fromJSONSchema` returns an untyped schema, so its output type is restored from the same generated schema.
const mutablePingJsonSchema: JSONSchema.JSONSchema = pingJsonSchema as unknown as Mutable<
	typeof pingJsonSchema
>;

export const pingSchema = z.fromJSONSchema(mutablePingJsonSchema) as z.ZodType<Ping>;

export function recordPing(pings: AnalyticsEngineDataset, ping: Ping): void {
	pings.writeDataPoint({
		indexes: [ping.anonymousId],
		blobs: [ping.event, ping.version, ping.channel, ping.os, ping.arch],
	});
}

export const postPing = defineHandler(
	zValidator('header', headerSchema),
	defineHandler.withValidator({ body: pingSchema })((c, { body }) => {
		recordPing(c.env.PINGS, body);
		return c.body(null, 204);
	}),
);

if (import.meta.vitest) {
	const app = new Hono<CloudEnv>().post('/v1/ping', async (c) => {
		const response = await postPing(c);
		assert.instanceOf(response, Response);
		return response;
	});

	const validPing = {
		event: 'dupin_daily_active',
		anonymousId: '4fd4c40cb4140e0e8952ceb2b3502e07',
		version: '0.4.0',
		channel: 'stable',
		os: 'macos 26',
		arch: 'aarch64',
	} as const satisfies Ping;

	describe('POST /v1/ping', () => {
		it('is the handler the route file exports as POST', async () => {
			const route = await import('../routes/v1/ping.ts');

			expect(route.POST).toBe(postPing);
		});

		it('stores only the documented fields of a valid ping', async () => {
			const writeDataPoint = vi.fn();
			const body = JSON.stringify({ ...validPing, ip: '203.0.113.1' });

			const response = await app.request(
				'/v1/ping',
				{
					method: 'POST',
					body,
					headers: { 'content-type': 'application/json', 'content-length': String(body.length) },
				},
				{ PINGS: { writeDataPoint } },
			);

			expect(response.status).toBe(204);
			expect(writeDataPoint.mock.calls).toEqual([
				[
					{
						indexes: [validPing.anonymousId],
						blobs: ['dupin_daily_active', '0.4.0', 'stable', 'macos 26', 'aarch64'],
					},
				],
			]);
		});

		it('accepts a nightly ping', async () => {
			const writeDataPoint = vi.fn();
			const body = JSON.stringify({ ...validPing, channel: 'nightly', version: '0.5.0-nightly.3' });

			const response = await app.request(
				'/v1/ping',
				{
					method: 'POST',
					body,
					headers: { 'content-type': 'application/json', 'content-length': String(body.length) },
				},
				{ PINGS: { writeDataPoint } },
			);

			expect(response.status).toBe(204);
			expect(writeDataPoint.mock.calls[0]?.[0]?.blobs).toEqual([
				'dupin_daily_active',
				'0.5.0-nightly.3',
				'nightly',
				'macos 26',
				'aarch64',
			]);
		});

		it.each([
			'{',
			'null',
			JSON.stringify({ ...validPing, event: 'other' }),
			JSON.stringify({ ...validPing, event: 'cli_daily_active' }),
			JSON.stringify({ ...validPing, anonymousId: 'not-hex' }),
			JSON.stringify({ ...validPing, anonymousId: validPing.anonymousId.toUpperCase() }),
			JSON.stringify({ ...validPing, channel: 'beta' }),
			JSON.stringify({ ...validPing, channel: 'Stable' }),
			JSON.stringify({ ...validPing, channel: undefined }),
			JSON.stringify({ ...validPing, os: '' }),
			JSON.stringify({ ...validPing, os: 'macos' }),
			JSON.stringify({ ...validPing, os: 'linux 6' }),
			JSON.stringify({ ...validPing, os: 'macos 26.1' }),
			JSON.stringify({ ...validPing, os: 'macos 1234' }),
			JSON.stringify({ ...validPing, arch: 'x86 64' }),
			JSON.stringify({ ...validPing, arch: 'x'.repeat(33) }),
			JSON.stringify({ ...validPing, arch: undefined }),
			JSON.stringify({ ...validPing, version: 'x'.repeat(65) }),
			JSON.stringify({ ...validPing, version: '0.4.0 beta' }),
			JSON.stringify({ ...validPing, version: 20 }),
		])('rejects the malformed ping %s without storing it', async (body) => {
			const writeDataPoint = vi.fn();

			const response = await app.request(
				'/v1/ping',
				{
					method: 'POST',
					body,
					headers: { 'content-type': 'application/json', 'content-length': String(body.length) },
				},
				{ PINGS: { writeDataPoint } },
			);

			expect(response.status).toBe(400);
			expect(writeDataPoint).not.toHaveBeenCalled();
		});

		it.each([
			{ 'content-type': 'text/plain', 'content-length': '153' },
			{ 'content-type': 'application/json', 'content-length': '2048' },
			{ 'content-type': 'application/json', 'content-length': 'abc' },
		])('rejects the headers %o before reading the body', async (headers) => {
			const writeDataPoint = vi.fn();

			const response = await app.request(
				'/v1/ping',
				{ method: 'POST', body: JSON.stringify(validPing), headers },
				{ PINGS: { writeDataPoint } },
			);

			expect(response.status).toBe(400);
			expect(writeDataPoint).not.toHaveBeenCalled();
		});
	});
}
