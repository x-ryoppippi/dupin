import { statSync } from 'node:fs';
import path from 'node:path';
import type { MarkdownNode, MarkdownTransformer } from '@ox-content/vite-plugin';
import { docRoute, githubUrl, isPublished } from './pages.ts';

const external = /^(?:[a-z][a-z0-9+.-]*:|\/\/|#)/i;

// The images the site serves, by extension, with their content types.
export const imageTypes: Readonly<Record<string, string>> = {
	'.png': 'image/png',
	'.webp': 'image/webp',
	'.svg': 'image/svg+xml',
	'.jpg': 'image/jpeg',
	'.jpeg': 'image/jpeg',
	'.gif': 'image/gif',
};

const isImage = (repoPath: string) => path.extname(repoPath).toLowerCase() in imageTypes;

// The repository path a relative link written in `document` points at, with its fragment, or
// nothing for an external link or one that leaves the repository.
function resolve(root: string, document: string, url: string) {
	if (!url || external.test(url)) {
		return undefined;
	}
	const [target, hash] = url.split(/(?=#)/, 2);
	const file = path.relative(root, path.resolve(path.dirname(document), target));
	if (file.startsWith('..')) {
		return undefined;
	}
	return { repoPath: file.split(path.sep).join('/'), fragment: hash ?? '' };
}

// Where the site serves an image of the repository: docs/images at its own path, and any other
// image under /docs/files/, since a GitHub page can't be embedded as an image.
export const imageRoute = (repoPath: string) =>
	repoPath.startsWith('docs/images/') ? `/${repoPath}` : `/docs/files/${repoPath}`;

// Where a link written in `document`, a file of the repository at `root`, goes on the site: a
// published page to its route, an image to the copy the site serves, and any other file of the
// repository, an unpublished page included, to GitHub.
export function siteUrl(root: string, document: string, url: string): string {
	const resolved = resolve(root, document, url);
	if (!resolved) {
		return url;
	}
	const { repoPath, fragment } = resolved;
	if (isImage(repoPath)) {
		return imageRoute(repoPath);
	}
	const docPath = repoPath.replace(/^docs\//, '');
	if (repoPath.startsWith('docs/') && isPublished(docPath)) {
		return docRoute(docPath) + fragment;
	}
	const directory = statSync(path.join(root, repoPath), { throwIfNoEntry: false })?.isDirectory();
	return githubUrl(repoPath, directory ? 'tree' : 'blob') + fragment;
}

const htmlUrl = /\b(src|srcset|href)="([^"]*)"/g;
const markdownUrl = /\]\(\s*<?([^)\s>]+)/g;

// The images of the repository outside docs/images that `source`, the text of `document`, links
// to or embeds, as repository paths, so the site can serve them.
export function linkedImages(root: string, document: string, source: string): string[] {
	const urls = [...source.matchAll(htmlUrl), ...source.matchAll(markdownUrl)].map((match) =>
		match.at(-1)!,
	);
	return urls.flatMap((url) => {
		const repoPath = resolve(root, document, url)?.repoPath;
		return repoPath && isImage(repoPath) && !repoPath.startsWith('docs/images/') ? [repoPath] : [];
	});
}

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
