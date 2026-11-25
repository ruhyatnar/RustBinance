<script lang="ts">
	import '../app.css';
	import { onMount, type Snippet } from 'svelte';
	import { page } from '$app/state';
	import { botStream } from '$lib/api/stream.svelte';
	import type { Venue } from '$lib/api/types';
	import AuthGate from '$lib/components/AuthGate.svelte';
	import ModeBanner from '$lib/components/ModeBanner.svelte';
	import VenueSwitchDialog, {
		type SwitchPolicyOption
	} from '$lib/components/VenueSwitchDialog.svelte';
	import VenueSwitcher from '$lib/components/VenueSwitcher.svelte';

	interface Props {
		children: Snippet;
	}

	let { children }: Props = $props();

	const navItems = [
		{ href: '/', label: 'Cockpit' },
		{ href: '/positions', label: 'Positions' },
		{ href: '/signals', label: 'Signals' },
		{ href: '/market', label: 'Market' },
		{ href: '/risk', label: 'Risk' },
		{ href: '/performance', label: 'Performance' },
		{ href: '/logs', label: 'Logs' },
		{ href: '/settings', label: 'Settings' },
		{ href: '/system', label: 'System' }
	];

	let actionFeedback = $state<string | null>(null);
	let switchDialogOpen = $state(false);
	let switchTarget = $state<Venue>('futures');

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
	<div class="flex min-h-screen flex-col bg-slate-950 text-slate-100">
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
		/>

		<nav class="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800 bg-slate-900/60 px-3 py-1.5">
			<div class="flex gap-1 overflow-x-auto">
				{#each navItems as item (item.href)}
					{@const active = page.url.pathname === item.href}
					<a
						href={item.href}
						class={`rounded-md px-3 py-1 text-xs font-semibold whitespace-nowrap transition ${
							active
								? 'bg-sky-600/25 text-sky-300 ring-1 ring-sky-500/50'
								: 'text-slate-400 hover:bg-slate-800 hover:text-slate-200'
						}`}
					>
						{item.label}
					</a>
				{/each}
			</div>

			{#if snap}
				<VenueSwitcher
					venues={snap.venues}
					readOnly={!botStream.canMutate}
					drainingOpenPositions={snap.positions.filter((p) => p.venue !== snap.venues.active).length}
					onRequestSwitch={handleRequestSwitch}
				/>
			{/if}
		</nav>

		{#if snap}
			<VenueSwitchDialog
				open={switchDialogOpen}
				currentVenue={snap.venues.active}
				targetVenue={switchTarget}
				engineMode={snap.engine.mode}
				openPositionsCount={snap.positions.filter((p) => p.venue === snap.venues.active).length}
				onConfirm={handleConfirmSwitch}
				onCancel={() => (switchDialogOpen = false)}
			/>
		{/if}

		{#if actionFeedback}
			<div class="flex items-center justify-between border-b border-amber-800/60 bg-amber-950/40 px-4 py-1.5 text-xs text-amber-200">
				<span>{actionFeedback}</span>
				<button type="button" onclick={() => (actionFeedback = null)} class="text-amber-400 hover:text-white">
					Dismiss
				</button>
			</div>
		{/if}

		<main class="mx-auto w-full max-w-[1600px] flex-1 p-4">
			{@render children()}
		</main>
	</div>
{/if}
