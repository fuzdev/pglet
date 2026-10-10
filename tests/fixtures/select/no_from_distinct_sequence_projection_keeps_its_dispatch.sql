-- The `DISTINCT` polarity of the **one exclusion** in the no-FROM quantifier lowering,
-- and the reason it is written down: the exclusion is a plan-shape fact with no other
-- reader, so it was guarded by nothing. Measured — dropping the
-- `&& !plan.is_sequence_projection()` reddens this fixture and no other in the whole
-- corpus, and `cargo test --workspace` stays green. The
-- executor dispatches `SELECT nextval/currval/setval(…)` on
-- `Plan::is_sequence_projection`, which matches a bare `Plan::Values`, so wrapping such a
-- projection in the `Plan::Distinct` the quantifier builds routes it to the read-only eval
-- path instead — where a `SequenceFn` fails loud `0A000` and this accepted statement
-- becomes an error.
--
-- Unlike the tail's `LIMIT`/`OFFSET` decline (`no_from_sequence_projection_keeps_its_dispatch.sql`),
-- this one changes no answer: a dedup over the single projected row returns that row, so
-- there is no residual divergence behind it. A fresh sequence keeps the value stable — the
-- statement advances the counter on both engines.
-- @setup
CREATE SEQUENCE nfdseq;
-- @test
SELECT DISTINCT nextval('nfdseq') AS n;
