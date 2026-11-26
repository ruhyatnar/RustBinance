<script lang="ts">
	import '@fontsource/manrope/400.css';
	import '@fontsource/manrope/500.css';
	import '@fontsource/manrope/600.css';
	import '@fontsource/manrope/700.css';
	import '@fontsource/inter/400.css';
	import '@fontsource/inter/500.css';
	import '@fontsource/inter/600.css';
	import '../app.css';
	import { onMount, type Snippet } from 'svelte';
	import { page } from '$app/state';
	import { botStream } from '$lib/api/stream.svelte';
	import type { Venue } from '$lib/api/types';
	import AuthGate from '$lib/components/AuthGate.svelte';
	import ModeBanner from '$lib/components/ModeBanner.svelte';
	import SidebarNav from '$lib/components/SidebarNav.svelte';
	import VenueSwitchDialog, {
		type SwitchPolicyOption
	} from '$lib/components/VenueSwitchDialog.svelte';
	import { sumDecimalStrings } from '$lib/format/decimal';

	interface Props {
		children: Snippet;
	}

	let { children }: Props = $props();

	let actionFeedback = $state<string | null>(null);
	let switchDialogOpen = $state(false);
	let switchTarget = $state<Venue>('futures');
	let mobileMenuOpen = $state(false);

	onMount(() => {
		void botStream.restoreFromSession();
	});

	async function handlePanic(): Promise<void> {
		try {
			const res = await botStream.executeConfirmedAction('panic_flatten');
			actionFeedback = `PANIC: ${res.message}`;
		} catch (err) {
			actionFeedback = err instanceof Error ? err.message : 'PANIC failed';
		}
	}

	function handleRequestSwitch(target: Venue): void {
		const active = botStream.snapshot?.venues.active ?? 'spot';
		if (target === active) return;
		switchTarget = target;
		switchDialogOpen = true;
		mobileMenuOpen = false;
	}

	async function handleConfirmSwitch(policy: SwitchPolicyOption): Promise<void> {
		switchDialogOpen = false;
		try {
			const res = await botStream.switchVenue(switchTarget, policy);
			actionFeedback = `VENUE SWITCH: ${res.message}`;
		} catch (err) {
			actionFeedback = err instanceof Error ? err.message : 'Venue switch failed';
		}
	}

	function handleToggleControls(): void {
		if (botStream.controlsUnlocked) {
			botStream.lockControls();
		} else {
			botStream.unlockControls();
		}
	}
</script>

{#if !botStream.authenticated}
	<AuthGate
		defaultUrl={botStream.baseUrl}
		errorMessage={botStream.lastError}
		onConnect={(url, tok, persist) => botStream.connect(url, tok, persist)}
	/>
{:else}
	{@const snap = botStream.snapshot}
	{@const drainingVenue =
		snap?.venues.spot === 'Draining'
			? 'Spot'
			: snap?.venues.futures === 'Draining'
				? 'Futures'
				: null}
	{@const drainingCount = snap
		? snap.positions.filter((p) => p.venue !== snap.venues.active).length
		: 0}

	<div class="flex min-h-screen flex-col bg-canvas text-ink">
		<ModeBanner
			buildMode={snap?.engine.build_mode ?? botStream.hello?.build_mode ?? 'paper'}
			venue={snap?.venues.active ?? 'spot'}
			engineMode={snap?.engine.mode ?? 'Starting'}
			equity={snap?.risk.equity ?? '0.00'}
			dayPnlPct={snap?.risk.day_pnl_pct ?? '0.00'}
			drawdownPct={snap?.risk.drawdown_pct ?? '0.00'}
			wsLatencyMs={snap?.engine.ws_latency_ms ?? 0}
			weightUsed={snap?.engine.rate.weight_used ?? 0}
			weightLimit={snap?.engine.rate.weight_limit ?? 6000}
			alertCount={snap?.alerts.filter((a) => !a.acked).length ?? 0}
			authScope={botStream.authScope}
			canMutate={botStream.canMutate}
			onPanic={handlePanic}
			onToggleControls={handleToggleControls}
			onDisconnect={() => botStream.disconnect()}
			onToggleMenu={() => (mobileMenuOpen = !mobileMenuOpen)}
		/>

		<div class="flex flex-1">
			<SidebarNav
				currentPath={page.url.pathname}
				venues={snap?.venues ?? null}
				canMutate={botStream.canMutate}
				drainingOpenPositions={drainingCount}
				open={mobileMenuOpen}
				onClose={() => (mobileMenuOpen = false)}
				onRequestSwitch={handleRequestSwitch}
			/>

			<div class="flex min-w-0 flex-1 flex-col">
				{#if drainingVenue}
					<div
						role="status"
						data-testid="global-draining-banner"
						class="border-b border-warning/40 bg-warning/10 px-4 py-2 font-mono text-xs text-warning"
					>
						⏳ {drainingVenue} sedang draining: {drainingCount} posisi dikelola, tidak ada entry baru
					</div>
				{/if}

				{#if actionFeedback}
					<div class="flex items-center justify-between border-b border-warning/40 bg-warning/10 px-4 py-2 font-mono text-xs text-warning">
						<span>{actionFeedback}</span>
						<button
							type="button"
							onclick={() => (actionFeedback = null)}
							class="rounded border border-warning/40 px-2 py-0.5 text-[11px] text-warning hover:bg-warning/20"
						>
							Dismiss
						</button>
					</div>
				{/if}

				<main class="mx-auto w-full max-w-[1680px] flex-1 p-4 lg:p-6">
					{@render children()}
				</main>
			</div>
		</div>

		{#if snap}
			{@const activeVenuePositions = snap.positions.filter((p) => p.venue === snap.venues.active)}
			<VenueSwitchDialog
				open={switchDialogOpen}
				currentVenue={snap.venues.active}
				targetVenue={switchTarget}
				engineMode={snap.engine.mode}
				openPositionsCount={activeVenuePositions.length}
				openPnlUsd={sumDecimalStrings(
					activeVenuePositions.map((p) => p.unrealized_quote),
					2
				)}
				onConfirm={handleConfirmSwitch}
				onCancel={() => (switchDialogOpen = false)}
			/>
		{/if}
	</div>
{/if}
