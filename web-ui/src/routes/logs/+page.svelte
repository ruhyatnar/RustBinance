<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import EventLog from '$lib/components/EventLog.svelte';

	let levelFilter = $state<string>('all');

	const filtered = $derived(
		levelFilter === 'all'
			? botStream.events
			: botStream.events.filter((e) => e.level === levelFilter)
	);
</script>

<div class="space-y-4">
	<div class="flex items-center gap-2 text-xs">
		<span class="text-slate-400">Filter level:</span>
		{#each ['all', 'info', 'warn', 'error', 'critical'] as lvl (lvl)}
			<button
				type="button"
				onclick={() => (levelFilter = lvl)}
				class={`rounded px-2.5 py-1 font-semibold uppercase ${
					levelFilter === lvl
						? 'bg-sky-600/30 text-sky-300 ring-1 ring-sky-500/50'
						: 'bg-slate-900 text-slate-400 hover:text-white'
				}`}
			>
				{lvl}
			</button>
		{/each}
	</div>

	<EventLog events={filtered} limit={200} />
</div>
