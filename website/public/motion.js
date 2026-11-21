const nav = document.querySelector('.nav');
const onScroll = () => nav?.classList.toggle('is-scrolled', window.scrollY > 12);
onScroll();
window.addEventListener('scroll', onScroll, { passive: true });

for (const button of document.querySelectorAll('[data-copy]')) {
	const status = button.parentElement?.querySelector('[data-copy-status]');
	let timer;
	button.addEventListener('click', async () => {
		try {
			await navigator.clipboard.writeText(button.dataset.copy);
		} catch {
			clearTimeout(timer);
			button.classList.remove('is-copied');
			if (status) status.textContent = 'Could not copy. Select the command to copy it instead.';
			return;
		}
		clearTimeout(timer);
		if (status) status.textContent = 'Copied the install command.';
		button.classList.add('is-copied');
		timer = setTimeout(() => {
			button.classList.remove('is-copied');
			if (status) status.textContent = '';
		}, 1400);
	});
}

const panel = document.querySelector('[data-scenarios]');
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

if (panel && !reducedMotion) {
	const scenarios = JSON.parse(panel.dataset.scenarios);
	const chip = panel.querySelector('.chip');
	const query = panel.querySelector('.query');
	const rows = panel.querySelector('.rows');
	const template = rows.querySelector('.row');
	const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

	let visible = true;
	new IntersectionObserver(([entry]) => {
		visible = entry.isIntersecting;
	}).observe(panel);
	const whileVisible = async () => {
		while (!visible || document.hidden) await sleep(400);
	};

	const renderRows = (list) => {
		rows.replaceChildren(
			...list.map((row, i) => {
				const li = template.cloneNode(true);
				li.className = i === 0 ? 'row is-selected' : 'row';
				li.querySelector('.app-icon').className = `app-icon app-icon--${row.icon}`;
				const title = li.querySelector('.row-title');
				title.replaceChildren();
				if (row.hit) {
					const mark = document.createElement('mark');
					mark.textContent = row.hit;
					title.append(mark, row.title.slice(row.hit.length));
				} else {
					title.textContent = row.title;
				}
				const sub = li.querySelector('.row-sub');
				sub.replaceChildren();
				if (row.alias) {
					const alias = document.createElement('span');
					alias.className = 'alias';
					alias.textContent = row.alias;
					sub.append(alias);
				}
				sub.append(row.sub);
				li.querySelector('.kbd')?.toggleAttribute('hidden', i !== 0);
				return li;
			}),
		);
	};

	const play = async () => {
		for (let i = 1; ; i = (i + 1) % scenarios.length) {
			await sleep(2600);
			await whileVisible();
			while (query.textContent.length > 0) {
				query.textContent = query.textContent.slice(0, -1);
				await sleep(45);
			}
			rows.replaceChildren();
			const scenario = scenarios[i];
			chip.hidden = !scenario.mode;
			chip.textContent = scenario.mode ?? '';
			await sleep(scenario.mode ? 420 : 200);
			for (const char of scenario.query) {
				query.textContent += char;
				await sleep(70 + Math.random() * 60);
			}
			await sleep(120);
			renderRows(scenario.rows);
		}
	};
	void play();
}
