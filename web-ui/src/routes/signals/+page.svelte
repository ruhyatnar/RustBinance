<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import SignalsFeed from '$lib/components/SignalsFeed.svelte';

	const snap = $derived(botStream.snapshot);
</script>

<div class="space-y-4">
	<SignalsFeed signals={snap?.signals_shadow ?? []} />
</div>
