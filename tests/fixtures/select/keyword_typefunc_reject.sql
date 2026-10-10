-- @expect-error: TYPE_FUNC_NAME keyword is not a valid bare column (ColId)
-- @no-error-fields: both engines send `P` and point at different tokens — 18.6 at the terminator, pglet at the keyword
-- `is` is TYPE_FUNC_NAME in `kwlist.h`; Postgres's `ColId` grammar excludes that
-- class, so a bare `is` in column position is a syntax error in Postgres — and
-- pglet matches it (rather than wrongly accepting a name PG rejects). The harness
-- regenerates the `@expect-error` expected shape from Postgres, so this asserts
-- pglet's SQLSTATE equals Postgres's `42601`, not merely "some error".

-- @test
SELECT is;
