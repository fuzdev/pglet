-- The refusal a scalar-subquery branch once hid was its **siblings'**: `integer` and
-- `date` have no common type whatever the subquery's is, and a branch set that deferred
-- on the subquery never compared them.
-- @setup
CREATE TABLE sshs (i INT, d DATE);
INSERT INTO sshs VALUES (1, '2020-01-02');
CREATE TABLE sshi (c INT);
-- @test
SELECT COALESCE((SELECT * FROM sshi), i, d) FROM sshs;
-- @expect-error: COALESCE types integer and date cannot be matched
