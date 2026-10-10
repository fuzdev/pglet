-- The accept polarity of the `duplicate_set_target_*` family: distinct target names,
-- one of them written quoted, all assigned in one statement. A gate keyed on the
-- spelling rather than on the column would refuse the quoted half of this list.
-- @setup
CREATE TABLE sldn (a INT, b INT, c INT);
INSERT INTO sldn VALUES (0, 0, 0);
UPDATE sldn SET a = 1, "b" = 2, c = 3;

-- @test
SELECT a, b, c FROM sldn;
