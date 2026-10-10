-- A radix-prefixed literal too large for any integer type is 18.6's `FCONST`, which
-- parse analysis makes a `numeric`; an integer accumulator would refuse it.
-- @test
SELECT 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF AS big, 0o17 AS oct, 0b1_0000 AS bin;
