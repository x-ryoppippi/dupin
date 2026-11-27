<script lang="ts" module>
	export const steps = [
		{ label: 'Hotkey', note: 'The panel appears' },
		{ label: 'A few letters', note: 'The same few reach the same place' },
		{ label: '↩', note: 'It opens' },
		{ label: 'Gone', note: 'Back to what you were doing' },
	];
</script>

<figure class="doc-loop">
	<ol>
		{#each steps as step, index (step.label)}
			<li class={{ last: index === steps.length - 1 }}>
				<span class="label">{step.label}</span>
				<span class="note">{step.note}</span>
			</li>
		{/each}
	</ol>
	<figcaption>The loop Dupin protects.</figcaption>
</figure>
