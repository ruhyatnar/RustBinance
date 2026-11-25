import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import AuthGate from './AuthGate.svelte';
import HoldToConfirm from './HoldToConfirm.svelte';
import ModeBanner from './ModeBanner.svelte';
import PositionTable from './PositionTable.svelte';
import type { PositionView } from '$lib/api/types';

describe('Safety & core UI components', () => {
	beforeEach(() => {
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it('HoldToConfirm requires 1.5s hold, aborts on early release, and ignores single keypress', async () => {
		const onConfirm = vi.fn();
		render(HoldToConfirm, {
			props: {
				label: 'PANIC FLATTEN',
				holdMs: 1500,
				onConfirm
			}
		});

		const btn = screen.getByRole('button', { name: /PANIC FLATTEN/i });

		// Single keypress MUST NEVER trigger panic (§11.1 #4)
		await fireEvent.keyDown(btn, { key: 'Enter' });
		await fireEvent.keyUp(btn, { key: 'Enter' });
		expect(onConfirm).not.toHaveBeenCalled();

		// Early release after 500ms MUST cancel
		await fireEvent.pointerDown(btn);
		await vi.advanceTimersByTimeAsync(500);
		await fireEvent.pointerUp(btn);
		await vi.advanceTimersByTimeAsync(1500);
		expect(onConfirm).not.toHaveBeenCalled();

		// Full 1500ms hold triggers exactly once
		await fireEvent.pointerDown(btn);
		await vi.advanceTimersByTimeAsync(1550);
		expect(onConfirm).toHaveBeenCalledTimes(1);
	});

	it('HoldToConfirm cancels pending hold timer on unmount and when disabled mid-hold', async () => {
		const onConfirm = vi.fn();
		const { rerender, unmount } = render(HoldToConfirm, {
			props: {
				label: 'PANIC FLATTEN',
				holdMs: 1500,
				disabled: false,
				onConfirm
			}
		});

		const btn = screen.getByRole('button', { name: /PANIC FLATTEN/i });
		await fireEvent.pointerDown(btn);
		await vi.advanceTimersByTimeAsync(500);
		await rerender({ label: 'PANIC FLATTEN', holdMs: 1500, disabled: true, onConfirm });
		await vi.advanceTimersByTimeAsync(1500);
		expect(onConfirm).not.toHaveBeenCalled();

		await rerender({ label: 'PANIC FLATTEN', holdMs: 1500, disabled: false, onConfirm });
		await fireEvent.pointerDown(btn);
		await vi.advanceTimersByTimeAsync(500);
		unmount();
		await vi.advanceTimersByTimeAsync(1500);
		expect(onConfirm).not.toHaveBeenCalled();
	});

	it('ModeBanner displays mode, venue, engine state, and hides PANIC in read-only mode', () => {
		const { rerender } = render(ModeBanner, {
			props: {
				buildMode: 'paper',
				venue: 'spot',
				engineMode: 'Ready',
				equity: '1012.40',
				dayPnlPct: '0.82',
				drawdownPct: '-1.1',
				wsLatencyMs: 14,
				weightUsed: 142,
				weightLimit: 6000,
				alertCount: 2,
				authScope: 'readonly',
				canMutate: false,
				onPanic: vi.fn(),
				onToggleControls: vi.fn(),
				onDisconnect: vi.fn()
			}
		});

		expect(screen.getByText('PAPER')).toBeInTheDocument();
		expect(screen.getByText('SPOT')).toBeInTheDocument();
		expect(screen.getByText('READY')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: /PANIC/i })).not.toBeInTheDocument();

		rerender({
			buildMode: 'live',
			venue: 'spot',
			engineMode: 'Ready',
			equity: '1012.40',
			dayPnlPct: '0.82',
			drawdownPct: '-1.1',
			wsLatencyMs: 14,
			weightUsed: 142,
			weightLimit: 6000,
			alertCount: 2,
			authScope: 'full',
			canMutate: true,
			onPanic: vi.fn(),
			onToggleControls: vi.fn(),
			onDisconnect: vi.fn()
		});

		expect(screen.getByText('LIVE')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: /PANIC/i })).toBeInTheDocument();
	});

	it('PositionTable renders venue badge, exact decimal strings, ▲/▼ icon, and hides Close in read-only', () => {
		const positions: PositionView[] = [
			{
				id: '0192f0a8-1234-7000-8000-000000000099',
				venue: 'spot',
				symbol: 'ETHUSDT',
				strategy: 'TrendPullback',
				state: 'OpenProtected',
				entry: '3120.40',
				mark: '3188.10',
				qty: '0.320',
				stop: '3098.00',
				tp1: '3200.00',
				tp2: '3280.00',
				profit_r: '1.9',
				unrealized_quote: '21.66',
				age_s: 15420,
				tp1_done: false,
				be_done: true
			}
		];

		render(PositionTable, {
			props: {
				positions,
				canMutate: false,
				onClosePosition: vi.fn()
			}
		});

		expect(screen.getByText('ETHUSDT')).toBeInTheDocument();
		expect(screen.getByText('SPOT')).toBeInTheDocument();
		expect(screen.getByText(/▲/)).toBeInTheDocument();
		expect(screen.getByText(/\+1\.9R/)).toBeInTheDocument();
		expect(screen.getByText('3,120.40')).toBeInTheDocument();
		expect(screen.getByText('BE')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: /Close/i })).not.toBeInTheDocument();
	});

	it('AuthGate submits baseUrl and bearer token without exposing data', async () => {
		const onConnect = vi.fn(async () => {});
		render(AuthGate, {
			props: {
				defaultUrl: 'http://127.0.0.1:3001',
				onConnect
			}
		});

		const tokenInput = screen.getByLabelText(/Bearer Token/i);
		await fireEvent.input(tokenInput, { target: { value: 'ro-token-123' } });
		const submit = screen.getByRole('button', { name: /Connect/i });
		await fireEvent.click(submit);

		expect(onConnect).toHaveBeenCalledWith('http://127.0.0.1:3001', 'ro-token-123', true);
	});
});
