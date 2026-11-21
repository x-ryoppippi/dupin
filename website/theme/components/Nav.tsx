import { docsUrl, repo } from '../site.ts';
import { GitHubIcon, Logo } from './icons.tsx';

export function Nav() {
	return (
		<header class="nav">
			<div class="nav-inner">
				<a class="brand" href="/">
					<Logo />
					<span>Dupin</span>
				</a>
				<nav class="nav-links">
					<a href="#install">Install</a>
					<a href={docsUrl}>Docs</a>
					<a href={repo} class="nav-gh">
						<GitHubIcon />
						<span>GitHub</span>
					</a>
				</nav>
			</div>
		</header>
	);
}
