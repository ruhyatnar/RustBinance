import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

vi.mock('$lib/api/stream.svelte', () => ({
	botStream: {
		connectionState: 'connected',
		canMutate: false,
		pause: vi.fn(),
		resume: vi.fn(),
		closePosition: vi.fn(),
		snapshot: {
			venues: {
				active: 'spot',
				spot: 'Active',
				futures: 'Disabled',
				last_switch_ms: 0,
				switches_today: 0
			},
			engine: {
				mode: 'Ready',
				build_mode: 'paper',
				uptime_s: 3600,
				ws_latency_ms: 12,
				clock_drift_ms: 1,
				rate: { weight_used: 120, weight_limit: 6000, orders_10s: 0 }
			},
			risk: {
				equity: '1012.40',
				day_pnl_pct: '0.82',
				week_pnl_pct: '2.10',
				drawdown_pct: '-1.10',
				heat_pct: '0.90',
				heat_max_pct: '2.00',
				open_positions: 1,
				max_positions: 4,
				btc_guard: 'SECURE',
				limits: { daily_halt_pct: '-2.0', kill_pct: '-10.0' }
			},
			positions: [
				{
					id: 'pos-spot-1',
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
					age_s: 1200,
					tp1_done: false,
					be_done: true
				}
			],
			orders: [],
			signals_shadow: [
				{
					venue: 'spot',
					symbol: 'SOLUSDT',
					strategy: 'Breakout',
					direction: 'Long',
					entry: '142.00',
					stop: '138.00',
					ts_ms: 1_700_000_000_000
				}
			],
			alerts: []
		},
		events: [
			{
				seq: 1,
				ts_ms: 1_700_000_000_000,
				venue: null,
				symbol: null,
				kind: 'engine',
				level: 'info',
				message: 'Engine ready globally'
			},
			{
				seq: 2,
				ts_ms: 1_700_000_001_000,
				venue: 'spot',
				symbol: 'ETHUSDT',
				kind: 'fill',
				level: 'info',
				message: 'Spot entry filled'
			}
		]
	}
}));

import CockpitPage from './+page.svelte';

describe('Cockpit page (Romer Hybrid)', () => {
	it('renders 4 KPI cards and filters venue widgets while keeping global events visible', async () => {
		render(CockpitPage);

		expect(screen.getByText('Equity')).toBeInTheDocument();
		expect(screen.getAllByText('Day PnL').length).toBeGreaterThanOrEqual(1);
		expect(screen.getAllByText('Portfolio Heat').length).toBeGreaterThanOrEqual(1);
		expect(screen.getAllByText('Drawdown').length).toBeGreaterThanOrEqual(1);

		expect(screen.getAllByText('ETHUSDT').length).toBeGreaterThanOrEqual(2);
		expect(screen.getByText('SOLUSDT')).toBeInTheDocument();
		expect(screen.getByText('Spot entry filled')).toBeInTheDocument();
		expect(screen.getByText('Engine ready globally')).toBeInTheDocument();
		expect(screen.getByText('Chart · ETHUSDT')).toBeInTheDocument();

		const futuresBtn = screen.getByRole('button', { name: /^Futures$/i });
		await fireEvent.click(futuresBtn);

		expect(screen.queryByText('ETHUSDT')).not.toBeInTheDocument();
		expect(screen.queryByText('SOLUSDT')).not.toBeInTheDocument();
		expect(screen.queryByText('Spot entry filled')).not.toBeInTheDocument();
		expect(screen.getByText('Engine ready globally')).toBeInTheDocument();
		expect(screen.getByText('Chart · BTCUSDT')).toBeInTheDocument();
	});
});
