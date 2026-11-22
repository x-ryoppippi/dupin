import { siteUrl } from './site.ts';

interface Env {
	ASSETS: { fetch(request: Request): Promise<Response> };
}

const canonical = new URL(siteUrl);

export default {
	fetch(request: Request, env: Env) {
		const url = new URL(request.url);
		if (url.protocol === 'http:' && url.hostname === canonical.hostname) {
			url.protocol = canonical.protocol;
			url.port = canonical.port;
			return Response.redirect(url.href, 308);
		}
		return env.ASSETS.fetch(request);
	},
};
