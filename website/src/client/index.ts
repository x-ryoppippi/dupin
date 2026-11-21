import '../styles/site.css';
import { hydrate } from 'svelte';
import { pages, type PageName } from '../pages/index.ts';

const app = document.getElementById('app');
const name = app?.dataset.page;
const props = document.getElementById('page-props')?.textContent;

if (app && name && name in pages && props) {
	hydrate(pages[name as PageName], { target: app, props: JSON.parse(props) });
}
