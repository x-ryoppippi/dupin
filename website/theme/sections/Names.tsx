import { Check } from '../components/Check.tsx';
import { Kbd } from '../components/Kbd.tsx';

export function Names() {
	return (
		<section class="band">
			<div class="container split">
				<div class="tile names">
					<div class="names-query">
						<span class="mono dim">query</span>
						<span class="mono">codex</span>
					</div>
					<div class="connector" aria-hidden="true">
						<span />
						<span />
						<span />
					</div>
					<div class="names-hit">
						<span class="app-icon app-icon--chatgpt" />
						<span class="row-text">
							<span class="row-title">ChatGPT</span>
							<span class="row-sub">
								<span class="alias">Codex</span>/Applications
							</span>
						</span>
						<Kbd>↩</Kbd>
					</div>
					<div class="names-list mono">
						<span>システム設定</span>
						<span>System Preferences</span>
						<span>browser → Safari</span>
					</div>
				</div>
				<div class="copy">
					<h2 class="display">Every name macOS knows.</h2>
					<p>
						An app matches by the name Finder shows, localized, by its file name, and by Spotlight's
						alternate names and keywords. Results learn what you pick for each query.
					</p>
					<ul class="checks">
						<Check>システム設定 on a Japanese Mac, System Settings everywhere</Check>
						<Check>ChatGPT answers to "Codex", its former name</Check>
						<Check>Ranked by frecency and by what you picked for a query</Check>
					</ul>
				</div>
			</div>
		</section>
	);
}
