-- A projection `unnest(…)`'s siblings are evaluated in target-list order at the first row,
-- a volatile one included: its division fails before the later sibling's overflow.
-- @setup
CREATE TABLE uo (i INT);
INSERT INTO uo VALUES (1), (2);
-- @test
-- @expect-error: division by zero
SELECT unnest(ARRAY[1]), (random() * 0)::int / 0, (SELECT i + 2147483647 FROM uo WHERE i = 2);
