<script lang="ts">
	import mark from '../../../assets/logo/dupin-mark.svg?raw';

	// The mark is square with the glasses across its middle; crop the viewBox to the glasses.
	const svg = mark
		.replace(/ width="\d+" height="\d+"/, '')
		.replace(/viewBox="[^"]*"/, 'viewBox="100 350 870 415" class="logo" aria-hidden="true"');
</script>

{@html svg}
