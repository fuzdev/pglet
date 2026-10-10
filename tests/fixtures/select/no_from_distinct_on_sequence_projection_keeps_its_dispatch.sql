-- The `DISTINCT ON` polarity of the same exclusion its `DISTINCT` sibling
-- (`no_from_distinct_sequence_projection_keeps_its_dispatch.sql`) pins. `DISTINCT ON`
-- wrapped a no-FROM sequence projection unconditionally, so this statement was `0A000`
-- here against 18.6's row; `plan_no_from_distinct_on` runs its per-key gates and then
-- declines to wrap, as `plan_no_from_order_limit` does. Measured — removing that decline
-- reddens this fixture and no other in the whole corpus, and `cargo test --workspace`
-- stays green.
--
-- The gates still fire ahead of the decline, so nothing loud became silent —
-- `DISTINCT ON (5)` over this projection is `42P10` and `DISTINCT ON (bogusdon)` is
-- `42703`, both matching 18.6. A fresh sequence keeps the value stable.
-- @setup
CREATE SEQUENCE nfdoseq;
-- @test
SELECT DISTINCT ON (1) nextval('nfdoseq') AS n;
