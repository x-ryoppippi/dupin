import type { Component } from 'svelte';
import { render } from 'svelte/server';
import DesignChoices from '../components/docs/DesignChoices.svelte';
import LatencyFigure from '../components/docs/LatencyFigure.svelte';
import LauncherLoop from '../components/docs/LauncherLoop.svelte';
import PanelHandoff from '../components/docs/PanelHandoff.svelte';
import WaysIn from '../components/docs/WaysIn.svelte';

const components: Record<string, Component> = {
	DesignChoices,
	LatencyFigure,
	LauncherLoop,
	PanelHandoff,
	WaysIn,
};

// A page in docs/ writes `<Name>` fallback `</Name>` for a component of the site. GitHub drops the
// unknown tag and shows the fallback; the site draws the component instead. The renderer leaves
// such a tag in its HTML as written, so this swaps it afterwards.
export function docComponents(html: string): string {
	return html.replace(/<([A-Z][A-Za-z]*)>[\s\S]*?<\/\1>/g, (tag, name: string) => {
		const component = components[name];
		return component ? render(component).body : tag;
	});
}
