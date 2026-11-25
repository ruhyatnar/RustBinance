<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import SignalsFeed from '$lib/components/SignalsFeed.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import { filterByVenue, type VenueFilter } from '$lib/filters/venue';

	const snap = $derived(botStream.snapshot);
	let venueFilter = $state<VenueFilter>('all');
	let symbolQuery = $state('');

	const filteredSignals = $derived.by(() => {
		const byVenue = snap ? filterByVenue(snap.signals_shadow, venueFilter) : [];
		const q = symbolQuery.trim().toUpperCase();
		if (!q) return byVenue;
		return byVenue.filter((s) => s.symbol.toUpperCase().includes(q));
	});
</script>

<div class="space-y-5">
	<PageHeader title="Signals" subtitle="Live and shadow strategy evaluations">
		{#snippet actions()}
			<input
				type="search"
				bind:value={symbolQuery}
				placeholder="Filter symbol…"
				aria-label="Filter symbol"
				class="w-36 rounded border border-hairline bg-surface-inset px-2.5 py-1 font-mono text-xs text-ink-strong uppercase placeholder:normal-case placeholder:text-ink-dim focus:border-accent focus:outline-none"
			/>
			<div
				class="inline-flex items-center rounded border border-hairline bg-surface-inset p-0.5 font-mono text-[11px]"
				role="group"
				aria-label="Filter venue"
			>
				{#each [
					{ id: 'all', label: 'All' },
					{ id: 'spot', label: 'Spot' },
					{ id: 'futures', label: 'Futures' }
				] as opt (opt.id)}
					<button
						type="button"
						aria-pressed={venueFilter === opt.id}
						onclick={() => (venueFilter = opt.id as VenueFilter)}
						class={`rounded px-2.5 py-1 font-semibold transition ${
							venueFilter === opt.id
								? 'bg-surface-elevated text-ink-strong ring-1 ring-accent/50'
								: 'text-ink-muted hover:text-ink'
						}`}
					>
						{opt.label}
					</button>
				{/each}
			</div>
		{/snippet}
	</PageHeader>

	<SignalsFeed signals={filteredSignals} />
</div>
