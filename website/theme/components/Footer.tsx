import { docsUrl, releasesUrl, repo, sponsorUrl } from '../site.ts';
import { Logo } from './icons.tsx';

export function Footer() {
	return (
		<footer class="footer">
			<div class="container footer-inner">
				<div class="footer-brand">
					<Logo />
					<span>© ryoppippi · GPL-3.0-or-later</span>
				</div>
				<nav class="footer-links">
					<a href={repo}>GitHub</a>
					<a href={docsUrl}>Docs</a>
					<a href={releasesUrl}>Releases</a>
					<a href={sponsorUrl}>Sponsor</a>
				</nav>
			</div>
		</footer>
	);
}
