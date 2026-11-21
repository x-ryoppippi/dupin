import { Kbd } from './Kbd.tsx';
import { SearchIcon } from './icons.tsx';
import { type Row, scenarios } from './scenarios.ts';

function PanelRow({ row, selected }: { row: Row; selected: boolean }) {
	return (
		<li class={selected ? 'row is-selected' : 'row'}>
			<span class={`app-icon app-icon--${row.icon}`} />
			<span class="row-text">
				<span class="row-title">
					{row.hit ? (
						<>
							<mark>{row.hit}</mark>
							{row.title.slice(row.hit.length)}
						</>
					) : (
						row.title
					)}
				</span>
				<span class="row-sub">
					{row.alias ? <span class="alias">{row.alias}</span> : ''}
					{row.sub}
				</span>
			</span>
			{selected ? <Kbd>↩</Kbd> : ''}
		</li>
	);
}

export function PanelDemo() {
	const [first] = scenarios;
	return (
		<div class="desk" data-rise style="--d: 300ms">
			<div class="panel" data-scenarios={JSON.stringify(scenarios)} aria-hidden="true">
				<div class="field">
					<SearchIcon />
					<span class="chip" hidden />
					<span class="query">{first.query}</span>
					<span class="caret" />
				</div>
				<ul class="rows">
					{first.rows.map((row, i) => (
						<PanelRow row={row} selected={i === 0} />
					))}
				</ul>
			</div>
		</div>
	);
}
