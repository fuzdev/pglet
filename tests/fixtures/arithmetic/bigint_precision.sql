-- 2^53 + 1 is not representable in f64; the old f64 arithmetic path returned
-- 9007199254740992. Integer-domain math keeps it exact.
-- @test
SELECT 9007199254740993 + 0 AS r;
