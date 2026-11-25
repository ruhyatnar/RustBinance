<script lang="ts">
	import type { Venue, VenuesSummary } from '$lib/api/types';
	import VenueSwitcher from './VenueSwitcher.svelte';

	interface Props {
		currentPath: string;
		venues?: VenuesSummary | null;
		canMutate: boolean;
		drainingOpenPositions: number;
		open: boolean;
		onClose: () => void;
		onRequestSwitch: (target: Venue) => void;
	}

	let {
		currentPath,
		venues = null,
		canMutate,
		drainingOpenPositions,
		open,
		onClose,
		onRequestSwitch
	}: Props = $props();

	interface NavItem {
		href: string;
		label: string;
		icon: string;
	}

	interface NavGroup {
		label: string;
		items: NavItem[];
	}

	const navGroups: NavGroup[] = [
		{
			label: 'Trading',
			items: [
				{ href: '/', label: 'Cockpit', icon: '◈' },
				{ href: '/positions', label: 'Positions', icon: '▤' },
				{ href: '/signals', label: 'Signals', icon: '⚡' },
				{ href: '/market', label: 'Market', icon: '◫' }
			]
		},
		{
			label: 'Analytics',
			items: [
				{ href: '/risk', label: 'Risk', icon: '◉' },
				{ href: '/performance', label: 'Performance', icon: '↗' }
			]
		},
		{
			label: 'System',
			items: [
				{ href: '/logs', label: 'Logs', icon: '≡' },
				{ href: '/settings', label: 'Settings', icon: '⚙' },
				{ href: '/system', label: 'System', icon: '◎' }
			]
		}
	];
</script>

{#snippet sidebarContent(isDrawer: boolean)}
	<div class="flex h-full flex-col justify-between">
		<div class="overflow-y-auto px-3 py-4">
			<div class="mb-5 flex items-center justify-between border-b border-hairline-subtle pb-3">
				<div>
					<div class="font-display text-sm font-semibold tracking-[-0.02em] text-ink-strong">
						◈ RUSTBINANCE
					</div>
					<div class="mt-0.5 font-mono text-[10px] tracking-wider text-ink-dim uppercase">
						Command Layer · IPC v1
					</div>
				</div>
				{#if isDrawer}
					<button
						type="button"
						onclick={onClose}
						aria-label="Close navigation"
						class="rounded border border-hairline bg-surface px-2 py-0.5 text-xs text-ink-muted hover:text-ink-strong"
					>
						✕
					</button>
				{/if}
			</div>

			<nav aria-label="Primary navigation" class="space-y-5">
				{#each navGroups as group (group.label)}
					<div>
						<div class="mb-1.5 px-2 font-mono text-[10px] font-semibold tracking-[0.12em] text-ink-dim uppercase">
							{group.label}
						</div>
						<ul class="space-y-0.5">
							{#each group.items as item (item.href)}
								{@const active = currentPath === item.href}
								<li>
									<a
										href={item.href}
										aria-current={active ? 'page' : undefined}
										onclick={() => {
											if (isDrawer) onClose();
										}}
										class={`flex items-center gap-2.5 rounded-r border-l-2 px-2.5 py-1.5 text-xs font-medium transition ${
											active
												? 'border-accent bg-surface-elevated text-ink-strong'
												: 'border-transparent text-ink-muted hover:bg-surface hover:text-ink'
										}`}
									>
										<span class="w-4 text-center font-mono text-[11px] opacity-80" aria-hidden="true">
											{item.icon}
										</span>
										<span>{item.label}</span>
									</a>
								</li>
							{/each}
						</ul>
					</div>
				{/each}
			</nav>
		</div>

		{#if venues && canMutate}
			<div class="border-t border-hairline-subtle p-3">
				<div class="mb-2 font-mono text-[10px] font-semibold tracking-[0.12em] text-ink-dim uppercase">
					Active Venue
				</div>
				<VenueSwitcher
					{venues}
					readOnly={!canMutate}
					{drainingOpenPositions}
					compact={true}
					{onRequestSwitch}
				/>
			</div>
		{/if}
	</div>
{/snippet}

<!-- Persistent desktop sidebar (lg+) -->
<aside class="hidden w-56 shrink-0 border-r border-hairline bg-sidebar lg:flex lg:flex-col">
	{@render sidebarContent(false)}
</aside>

<!-- Mobile / tablet slide-over drawer (< lg) -->
{#if open}
	<div class="fixed inset-0 z-50 flex lg:hidden" role="dialog" aria-modal="true" aria-label="Navigation menu">
		<button
			type="button"
			aria-label="Close navigation backdrop"
			onclick={onClose}
			class="fixed inset-0 bg-black/70 backdrop-blur-xs"
		></button>
		<aside class="relative z-10 flex w-64 max-w-[80vw] flex-col border-r border-hairline bg-sidebar shadow-2xl">
			{@render sidebarContent(true)}
		</aside>
	</div>
{/if}
