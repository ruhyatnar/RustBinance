<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import EventLog from '$lib/components/EventLog.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';

	let levelFilter = $state<string>('all');

	const filtered = $derived(
		levelFilter === 'all'
			? botStream.events
			: botStream.events.filter((e) => e.level === levelFilter)
	);
</script>

<div class="space-y-5">
	<PageHeader title="Journal & Logs" subtitle="Live structured engine event stream">
		{#snippet actions()}
			<div
				class="inline-flex items-center rounded border border-hairline bg-surface-inset p-0.5 font-mono text-[11px]"
				role="group"
				aria-label="Filter log level"
			>
				{#each ['all', 'info', 'warn', 'error', 'critical'] as lvl (lvl)}
					<button
						type="button"
						aria-pressed={levelFilter === lvl}
						onclick={() => (levelFilter = lvl)}
						class={`rounded px-2.5 py-1 font-semibold uppercase transition ${
							levelFilter === lvl
								? 'bg-surface-elevated text-ink-strong ring-1 ring-accent/50'
								: 'text-ink-muted hover:text-ink'
						}`}
					>
						{lvl}
					</button>
				{/each}
			</div>
		{/snippet}
	</PageHeader>

	<EventLog events={filtered} limit={200} />
</div>
