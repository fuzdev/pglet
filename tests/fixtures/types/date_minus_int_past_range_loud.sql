-- `date - integer` past 18.6's own range (4714-11-24 BC) is its `date out of range`,
-- 22008, and pglet's date domain answers the same sentence under the same code.
-- @expect-error: date out of range
-- @expect-message
-- @test
SELECT '0001-01-01'::date - 2147483647;
