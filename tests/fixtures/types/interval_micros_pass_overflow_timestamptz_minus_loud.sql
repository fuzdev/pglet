-- The microseconds pass overflows `i64`, and the sum wrapped round would land inside the
-- date domain, so the overflow check is what refuses it.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamptz '9999-12-31 00:00:00+00' - interval '-103830043 days -2562047788:00:54.775807';
