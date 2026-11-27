<script lang="ts" module>
	export const flow = ['Query', 'Sources', 'Filters', 'Rows', 'Kinds', 'Actions'];
</script>

<figure class="doc-pipeline">
	<ol class="flow">
		{#each flow as step (step)}
			<li>{step}</li>
		{/each}
	</ol>
	<div class="stages">
		<div class="box">
			<span class="label">Sources</span>
			<span class="note">Produce rows for the query.</span>
			<span class="part">In the core</span>
			<span class="note">Apps, Files, folder browsing and Recent items.</span>
			<div class="ext">
				<span class="part">An extension adds</span>
				<span class="note"
					>Its own keyword, answered by <code>search(query)</code> or an <code>items()</code> catalogue,
					which can join the main search too.</span
				>
				<span class="gate">Needs your switch, and each scope it asks for.</span>
			</div>
		</div>
		<div class="box">
			<span class="label">Filters</span>
			<span class="note">Match the rows and put them in order.</span>
			<span class="part">In the core</span>
			<span class="note">The fuzzy and prefix matchers, and the relevance and title sorters.</span>
			<div class="ext">
				<span class="part">An extension adds</span>
				<span class="note">A matcher or a sorter for apps or a browsed folder.</span>
				<span class="gate"
					>Needs <code>rows:app</code> or <code>rows:file</code>, and <code>query</code> for a matcher.</span
				>
			</div>
		</div>
		<div class="box">
			<span class="label">Kinds</span>
			<span class="note">Give each row the actions for its type.</span>
			<span class="part">In the core</span>
			<span class="note"
				>The text, URL, file and app kinds. A file offers Open, Reveal in Finder, Quick Look and more.</span
			>
			<div class="ext">
				<span class="part">An extension adds</span>
				<span class="note">An action on every row of a kind that Dupin’s own sources found.</span>
				<span class="gate">Needs <code>rows:&lt;kind&gt;</code>, with the one field it takes.</span>
			</div>
		</div>
	</div>
	<div class="footer">
		<p class="note">
			The core stays small: it owns the pipeline, the command registry, rendering, permissions and the
			native sources and kinds. Built-ins that need no native API, such as Calculator and Clipboard
			history, ship as bundled extensions.
		</p>
		<p class="note">
			<code>[advanced]</code> in <code>config.toml</code> lets you recompose the stages: a source’s matcher
			and sorter, what the main search also searches, and the actions a kind adds.
		</p>
	</div>
	<figcaption>
		One pipeline for every search, after Shougo Matsushita’s unite.vim, denite.nvim and ddu.vim.
	</figcaption>
</figure>
