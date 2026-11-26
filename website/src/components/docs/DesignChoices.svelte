<script lang="ts">
	const choices = [
		{
			title: 'A doorway, not a replacement',
			text: 'The launcher gets you to your desktop; the desktop is where work happens.',
		},
		{
			title: 'The panel is transient',
			text: 'It appears, you act, it vanishes. Windows keep what you want to keep.',
		},
		{
			title: 'Same keys everywhere',
			text: 'What you learn once works in every search and every extension.',
		},
		{
			title: 'Grows by extensions',
			text: 'What falls outside a launcher’s job can be an extension, not part of the core.',
		},
	];
</script>

<ul class="doc-choices">
	{#each choices as choice (choice.title)}
		<li>
			<strong>{choice.title}</strong>
			<span>{choice.text}</span>
		</li>
	{/each}
</ul>
