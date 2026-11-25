<script lang="ts">
	import { onDestroy } from 'svelte';

	interface Props {
		label: string;
		holdMs?: number;
		disabled?: boolean;
		variant?: 'danger' | 'warning';
		onConfirm: () => void | Promise<void>;
	}

	let {
		label,
		holdMs = 1500,
		disabled = false,
		variant = 'danger',
		onConfirm
	}: Props = $props();

	let holding = $state(false);
	let progressPct = $state(0);
	let busy = $state(false);

	let startAt = 0;
	let tickTimer: ReturnType<typeof setInterval> | null = null;
	let fireTimer: ReturnType<typeof setTimeout> | null = null;

	function cancelHold(): void {
		holding = false;
		progressPct = 0;
		if (tickTimer !== null) {
			clearInterval(tickTimer);
			tickTimer = null;
		}
		if (fireTimer !== null) {
			clearTimeout(fireTimer);
			fireTimer = null;
		}
	}

	$effect(() => {
		if (disabled) {
			cancelHold();
		}
	});

	onDestroy(() => {
		cancelHold();
	});

	function startHold(): void {
		if (disabled || busy || holding) return;
		holding = true;
		progressPct = 0;
		startAt = Date.now();

		tickTimer = setInterval(() => {
			const elapsed = Date.now() - startAt;
			progressPct = Math.min(100, Math.round((elapsed / holdMs) * 100));
		}, 50);

		fireTimer = setTimeout(async () => {
			cancelHold();
			busy = true;
			try {
				await onConfirm();
			} finally {
				busy = false;
			}
		}, holdMs);
	}

	function handleKeyDown(ev: KeyboardEvent): void {
		// Intentionally block single-key shortcuts for destructive actions (§11.1 #4).
		if (ev.key === 'Enter' || ev.key === ' ') {
			ev.preventDefault();
		}
	}

	const colorClass = $derived(
		variant === 'danger'
			? 'border-danger-solid/80 bg-danger-solid/90 hover:bg-danger-solid text-white'
			: 'border-warning/70 bg-warning/90 hover:bg-warning text-canvas'
	);
</script>

<button
	type="button"
	disabled={disabled || busy}
	onpointerdown={startHold}
	onpointerup={cancelHold}
	onpointerleave={cancelHold}
	onpointercancel={cancelHold}
	onkeydown={handleKeyDown}
	aria-label={`${label} (hold ${(holdMs / 1000).toFixed(1)}s)`}
	class={`relative overflow-hidden rounded border px-3 py-1 font-mono text-[11px] font-bold tracking-wider uppercase select-none transition disabled:cursor-not-allowed disabled:opacity-45 ${colorClass}`}
>
	<span
		class="pointer-events-none absolute inset-y-0 left-0 bg-white/30 transition-[width] duration-75"
		style={`width: ${progressPct}%`}
	></span>
	<span class="relative z-10 flex items-center gap-1.5">
		<span aria-hidden="true">■</span>
		<span>{busy ? 'Executing…' : holding ? `Hold ${progressPct}%` : `${label} (hold)`}</span>
	</span>
</button>
