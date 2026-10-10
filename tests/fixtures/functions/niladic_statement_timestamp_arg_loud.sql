-- `statement_timestamp`, the third of the four `now`-family spellings. They evaluate
-- identically in pglet (no transaction- or statement-time source — a recorded
-- divergence), which is exactly why the refusal has to be asserted per name.
-- @test
-- @expect-error: function statement_timestamp(integer) does not exist
SELECT statement_timestamp(1);
