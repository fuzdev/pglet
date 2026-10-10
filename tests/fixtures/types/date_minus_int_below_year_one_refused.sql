-- Below 0001-01-01 is outside pglet's date domain, which models no BC: 18.6 answers
-- `0001-12-31 BC` and pglet refuses, `22008 date out of range`. 18.6 accepts, so this
-- fixture holds only that pglet *errors* — any code passes it. The code is pinned by
-- `executor::expr`'s arithmetic tests and `crates/pglet/tests/temporal_domain.rs`.
-- Accepted, it would store and dump `0000-12-31`, a row a restore into 18.6 drops.
-- @expect-error: pglet refuses a BC date
-- @test
SELECT ('0001-01-01'::date - 1)::text;
