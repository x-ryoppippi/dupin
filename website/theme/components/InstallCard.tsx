import { installCommand } from '../site.ts';
import { CopyIcon } from './icons.tsx';

export function InstallCard() {
	return (
		<div class="install-card" data-rise style="--d: 200ms">
			<div class="install-row">
				<span class="install-label">Nix</span>
				<code class="install-value">{installCommand}</code>
				<button
					class="copy-button"
					type="button"
					data-copy={installCommand}
					aria-label="Copy the install command"
				>
					<CopyIcon />
				</button>
				<span class="visually-hidden" role="status" data-copy-status />
			</div>
			<div class="install-row">
				<span class="install-label">Requires</span>
				<span class="install-value install-value--plain">Apple silicon, macOS 14 or later</span>
				<a class="install-more" href="#install">
					Setup <span class="arrow">↓</span>
				</a>
			</div>
		</div>
	);
}
