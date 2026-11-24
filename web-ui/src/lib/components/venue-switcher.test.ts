import { describe, expect, it } from 'vitest';
import type { VenuesSummary } from '$lib/api/types';

describe('VenueSwitcher & VenueSwitchDialog contract (§11.6)', () => {
	it('represents Spot Draining + Futures Active cleanly in VenuesSummary', () => {
		const summary: VenuesSummary = {
			active: 'UsdsFutures',
			spot: 'Draining',
			futures: 'Active',
			last_switch_ms: 1_700_000_000_000,
			switches_today: 1
		};
		expect(summary.active).toBe('UsdsFutures');
		expect(summary.spot).toBe('Draining');
		expect(summary.futures).toBe('Active');
	});
});
