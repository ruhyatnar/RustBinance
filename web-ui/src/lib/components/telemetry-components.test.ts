import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { RiskSummary, ShadowSignalView } from '$lib/api/types';
import type { LoggedEvent } from '$lib/api/stream.svelte';
import EventLog from './EventLog.svelte';
import RiskGauge from './RiskGauge.svelte';
import SignalsFeed from './SignalsFeed.svelte';

describe('Telemetry components (Romer theme)', () => {
	it('RiskGauge renders BTC Guard pill, aggregate subtitle, and ratios', () => {
		const risk: RiskSummary = {
			equity: '1012.40',
			day_pnl_pct: '0.82',
			week_pnl_pct: '2.15',
			drawdown_pct: '-1.10',
			heat_pct: '0.90',
			heat_max_pct: '2.00',
			open_positions: 2,
			max_positions: 4,
			btc_guard: 'SECURE',
			limits: { daily_halt_pct: '-2.0', kill_pct: '-10.0' }
		};

		render(RiskGauge, { props: { risk } });

		expect(screen.getByText('Account Aggregate')).toBeInTheDocument();
		expect(screen.getByText(/BTC Guard: SECURE/i)).toBeInTheDocument();
		expect(screen.getByText('2 / 4')).toBeInTheDocument();
	});

	it('EventLog renders global (venue: null) and venue-tagged events cleanly', () => {
		const events: LoggedEvent[] = [
			{
				seq: 1,
				ts_ms: 1_700_000_000_000,
				venue: null,
				symbol: null,
				kind: 'engine',
				level: 'warn',
				message: 'BTC guard triggered'
			},
			{
				seq: 2,
				ts_ms: 1_700_000_001_000,
				venue: 'futures',
				symbol: 'ETHUSDT',
				kind: 'fill',
				level: 'info',
				message: 'Entry filled'
			}
		];

		render(EventLog, { props: { events } });
		expect(screen.getByText('BTC guard triggered')).toBeInTheDocument();
		expect(screen.getByText('ETHUSDT')).toBeInTheDocument();
		expect(screen.getByText('futures')).toBeInTheDocument();
	});

	it('SignalsFeed renders venue badge, symbol, strategy, direction, and prices', () => {
		const signals: ShadowSignalView[] = [
			{
				venue: 'spot',
				symbol: 'SOLUSDT',
				strategy: 'TrendPullback',
				direction: 'Long',
				entry: '142.50',
				stop: '138.00',
				ts_ms: 1_700_000_000_000
			}
		];

		render(SignalsFeed, { props: { signals } });
		expect(screen.getByText('SOLUSDT')).toBeInTheDocument();
		expect(screen.getByText('TrendPullback')).toBeInTheDocument();
		expect(screen.getByText('Long')).toBeInTheDocument();
		expect(screen.getByText(/142\.50/)).toBeInTheDocument();
	});
});
