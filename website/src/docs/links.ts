import { statSync } from 'node:fs';
import path from 'node:path';
import type { MarkdownNode, MarkdownTransformer } from '@ox-content/vite-plugin';
import { docRoute, githubUrl, isPublished } from './pages.ts';

const external = /^(?:[a-z][a-z0-9+.-]*:|\/\/|#)/i;

// Where a link written in `document`, a file of the repository at `root`, goes on the site: a
// published page to its route, an image in docs/images to the copy the site serves, and any other
// file of the repository, an unpublished page included, to GitHub.
export function siteUrl(root: string, document: string, url: string): string {
	if (!url || external.test(url)) {
		return url;
	}
	const [target, hash] = url.split(/(?=#)/, 2);
	const file = path.relative(root, path.resolve(path.dirname(document), target));
	if (file.startsWith('..')) {
		return url;
	}
	const repoPath = file.split(path.sep).join('/');
	const fragment = hash ?? '';
	if (repoPath.startsWith('docs/images/')) {
		return `/${repoPath}`;
	}
	const docPath = repoPath.replace(/^docs\//, '');
	if (repoPath.startsWith('docs/') && isPublished(docPath)) {
		return docRoute(docPath) + fragment;
	}
	const directory = statSync(path.join(root, file), { throwIfNoEntry: false })?.isDirectory();
	return githubUrl(repoPath, directory ? 'tree' : 'blob') + fragment;
}

const htmlUrl = /\b(src|srcset|href)="([^"]*)"/g;

// Rewrites every link, image and raw-HTML `src`, `srcset` and `href` in the docs to `siteUrl`,
// before ox-content would turn `.md` links into relative paths of its own.
export function docsLinks(root: string): MarkdownTransformer {
	return {
		name: 'dupin-docs-links',
		transform(ast, { filePath }) {
			const visit = (node: MarkdownNode): void => {
				if (typeof node.url === 'string') {
					node.url = siteUrl(root, filePath, node.url);
				}
				if (node.type === 'html' && typeof node.value === 'string') {
					node.value = node.value.replace(
						htmlUrl,
						(_, attribute: string, url: string) => `${attribute}="${siteUrl(root, filePath, url)}"`,
					);
				}
				node.children?.forEach(visit);
			};
			visit(ast);
			return ast;
		},
	};
}
