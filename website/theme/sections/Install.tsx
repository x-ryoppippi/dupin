import { raw, usePageProps } from '@ox-content/vite-plugin';

// The steps live in content/index.md, so they are edited as Markdown.
export function Install() {
	const page = usePageProps();
	return (
		<section class="band install">
			<div class="container install-inner">
				<div class="prose">{raw(page.html)}</div>
			</div>
		</section>
	);
}
