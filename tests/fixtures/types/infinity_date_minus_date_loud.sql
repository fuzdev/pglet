-- `date - date` refuses an infinite operand on both engines, with `date_mi`'s sentence.
-- @expect-error: cannot subtract infinite dates
-- @expect-message
-- @test
SELECT date 'infinity' - date '2000-01-01';
