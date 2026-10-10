-- A well-formed numeric literal past the format weight cap fails loud at parse
-- (Postgres `numeric_in` runs `make_result`, raising "value overflows numeric
-- format", 22003) rather than silently constructing an over-cap value. `1e131072`
-- is one power of ten past the largest representable numeric.
-- @test
-- @expect-error: numeric literal weight overflow
-- @expect-message
SELECT '1e131072'::numeric;
