import { Check } from '../components/Check.tsx';
import { Kbd } from '../components/Kbd.tsx';

function ConfigTile() {
	return (
		<div class="tile term">
			<div class="term-bar" aria-hidden="true">
				<span />
				<span />
				<span />
				<span class="term-title mono">~/.config/dupin/config.toml</span>
			</div>
			<pre class="term-body mono">
				<span class="t-key">hotkey</span> = <span class="t-str">"cmd+space"</span>
				{'\n'}
				<span class="t-key">position</span> = <span class="t-str">"center"</span>
				{'\n'}
				<span class="t-key">input_source</span> = <span class="t-str">"ascii"</span>
				{'\n'}
				<span class="t-key">file_limit</span> = <span class="t-num">200</span>
				<span class="block-caret" />
			</pre>
		</div>
	);
}

export function OutOfTheWay() {
	return (
		<section class="band">
			<div class="container split split--flip">
				<div class="copy">
					<h2 class="display">Stays out of the way.</h2>
					<p>
						No Dock icon, no menu bar item, no window until you ask. The panel is drawn ahead of
						time while hidden, and gone again on Esc or a click elsewhere.
					</p>
					<ul class="checks">
						<Check>
							Switches to an ASCII input source while it shows, so IMEs never hold the query
						</Check>
						<Check>Says why when the hotkey is taken, instead of failing silently</Check>
						<Check>
							One <code>config.toml</code>, or <Kbd>⌘</Kbd>
							<Kbd>,</Kbd> for Settings
						</Check>
					</ul>
				</div>
				<ConfigTile />
			</div>
		</section>
	);
}
