//! Deterministic double round-robin via the circle method. Pure function of
//! the club ordering — shuffle the input (seeded) if variety is wanted.

use fforge_domain::{ClubId, Fixture, FixtureId};

/// Requires an even number of clubs (≥ 2). Produces 2·(n−1) matchdays; the
/// second half mirrors the first with venues swapped. Fixture ids are
/// sequential in emission order.
pub fn double_round_robin(clubs: &[ClubId]) -> Vec<Fixture> {
    let n = clubs.len();
    assert!(n >= 2 && n % 2 == 0, "need an even number of clubs, got {n}");
    let rounds = (n - 1) as u8;
    let mut fixtures = Vec::with_capacity(n * (n - 1));
    let mut next_id = 0u32;

    // Circle method: club[0] fixed, the rest rotate.
    let mut ring: Vec<ClubId> = clubs[1..].to_vec();

    let mut first_half: Vec<Vec<(ClubId, ClubId)>> = Vec::with_capacity(rounds as usize);
    for round in 0..rounds {
        let mut pairs = Vec::with_capacity(n / 2);
        // Alternate the fixed club's venue so nobody plays 19 straight at home.
        let (a, b) = (clubs[0], ring[0]);
        if round % 2 == 0 {
            pairs.push((a, b));
        } else {
            pairs.push((b, a));
        }
        for i in 1..n / 2 {
            let x = ring[i];
            let y = ring[n - 1 - i];
            if round % 2 == 0 {
                pairs.push((y, x));
            } else {
                pairs.push((x, y));
            }
        }
        first_half.push(pairs);
        ring.rotate_right(1);
    }

    for (round, pairs) in first_half.iter().enumerate() {
        for &(home, away) in pairs {
            fixtures.push(Fixture {
                id: FixtureId(next_id),
                matchday: round as u8 + 1,
                home,
                away,
            });
            next_id += 1;
        }
    }
    // Second half: mirror with venues swapped.
    for (round, pairs) in first_half.iter().enumerate() {
        for &(home, away) in pairs {
            fixtures.push(Fixture {
                id: FixtureId(next_id),
                matchday: rounds + round as u8 + 1,
                home: away,
                away: home,
            });
            next_id += 1;
        }
    }
    fixtures
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn round_robin_properties() {
        let clubs: Vec<ClubId> = (0..20).map(ClubId).collect();
        let fixtures = double_round_robin(&clubs);
        assert_eq!(fixtures.len(), 20 * 19);

        // Every ordered pair appears exactly once (each pair home & away).
        let mut seen: BTreeMap<(ClubId, ClubId), u32> = BTreeMap::new();
        for f in &fixtures {
            assert_ne!(f.home, f.away);
            *seen.entry((f.home, f.away)).or_default() += 1;
        }
        assert!(seen.values().all(|&c| c == 1));
        assert_eq!(seen.len(), 20 * 19);

        // Each club plays exactly once per matchday.
        for md in 1..=38u8 {
            let mut playing = Vec::new();
            for f in fixtures.iter().filter(|f| f.matchday == md) {
                playing.push(f.home);
                playing.push(f.away);
            }
            playing.sort();
            playing.dedup();
            assert_eq!(playing.len(), 20, "matchday {md}");
        }
    }
}