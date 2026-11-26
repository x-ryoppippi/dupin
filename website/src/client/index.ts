import '../styles/site.css';
import { hydrate, mount } from 'svelte';
import DocsSearch from '../components/DocsSearch.svelte';
import { pages, type PageName } from '../pages/index.ts';

const app = document.getElementById('app');
const name = app?.dataset.page;
const props = document.getElementById('page-props')?.textContent;

if (app && name && name in pages && props) {
	hydrate(pages[name as PageName], { target: app, props: JSON.parse(props) });
}

// A docs page is static HTML apart from its search box.
const search = document.getElementById('docs-search');
const searchPages = document.getElementById('docs-pages')?.textContent;
if (search && searchPages) {
	mount(DocsSearch, { target: search, props: { pages: JSON.parse(searchPages) } });
}
