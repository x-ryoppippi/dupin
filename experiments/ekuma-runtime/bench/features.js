// What the bundled calendar extension needs the host to do for it today, done in the guest.
const instant = new Date(Date.UTC(2026, 10, 26, 9, 30));

function probe(title, run) {
	try {
		return { title, text: String(run()) };
	} catch (error) {
		return { title, text: `${error.name}: ${error.message}` };
	}
}

export function search() {
	return [
		probe('typeof Intl', () => typeof Intl),
		probe('typeof Temporal', () => typeof Temporal),
		probe('DateTimeFormat ja-JP, Asia/Tokyo', () =>
			new Intl.DateTimeFormat('ja-JP', {
				dateStyle: 'full',
				timeStyle: 'short',
				timeZone: 'Asia/Tokyo',
			}).format(instant),
		),
		probe('DateTimeFormat en-GB, Europe/London', () =>
			new Intl.DateTimeFormat('en-GB', {
				dateStyle: 'full',
				timeStyle: 'short',
				timeZone: 'Europe/London',
			}).format(instant),
		),
		probe('DateTimeFormat default locale and zone', () => {
			const options = new Intl.DateTimeFormat().resolvedOptions();
			return `${options.locale} ${options.timeZone}`;
		}),
		probe('formatRange en-GB', () =>
			new Intl.DateTimeFormat('en-GB', { timeStyle: 'short', timeZone: 'UTC' }).formatRange(
				instant,
				new Date(instant.getTime() + 3_600_000),
			),
		),
		probe('RelativeTimeFormat ja', () =>
			new Intl.RelativeTimeFormat('ja', { numeric: 'auto' }).format(1, 'day'),
		),
		probe('Collator ja sort', () =>
			['さくら', 'あおい', 'カエデ'].sort(new Intl.Collator('ja').compare).join(','),
		),
		probe('Segmenter ja words', () =>
			[...new Intl.Segmenter('ja', { granularity: 'word' }).segment('今日は良い天気です')]
				.map((part) => part.segment)
				.join('|'),
		),
		probe('NumberFormat en-GB GBP', () =>
			new Intl.NumberFormat('en-GB', { style: 'currency', currency: 'GBP' }).format(1234.5),
		),
		probe('Temporal.Now zone', () => Temporal.Now.timeZoneId()),
		probe('Temporal PlainDate + 1 month', () =>
			Temporal.PlainDate.from('2026-01-31').add({ months: 1 }).toString(),
		),
		probe('Temporal ZonedDateTime Tokyo', () =>
			Temporal.Instant.from('2026-11-26T09:30:00Z').toZonedDateTimeISO('Asia/Tokyo').toString(),
		),
		probe('Temporal japanese calendar', () =>
			Temporal.PlainDate.from('2026-11-26')
				.withCalendar('japanese')
				.toLocaleString('ja-JP-u-ca-japanese', {
					era: 'long',
					year: 'numeric',
				}),
		),
		probe('Date toLocaleDateString ja-JP', () =>
			instant.toLocaleDateString('ja-JP', { timeZone: 'Asia/Tokyo' }),
		),
	];
}
