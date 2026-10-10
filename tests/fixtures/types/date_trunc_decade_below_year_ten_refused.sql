-- `date_trunc('decade', …)` over years 1 through 9 is year 0: 18.6 answers
-- `0001-01-01 00:00:00 BC` and pglet, which models no BC, refuses
-- `22008 timestamp out of range`. 18.6 accepts, so this fixture holds only that pglet
-- errors; the code is pinned by `executor::expr`'s `date_trunc_holds_the_domain`.
-- @expect-error: pglet refuses a BC timestamp
-- @test
SELECT date_trunc('decade', timestamp '0005-06-01')::text;
