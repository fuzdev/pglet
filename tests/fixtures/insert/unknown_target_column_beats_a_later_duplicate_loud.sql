-- The accept-side half of the interleave: an unknown name at item 1 is reached before
-- the repeat behind it, so the answer is `42703` and not `42701`. Both engines agreed
-- here before the duplicate gate existed, and a gate written as a whole-list pass ahead
-- of the resolution would have reddened this file rather than its sibling
-- `duplicate_target_column_beats_a_later_unknown_name_loud.sql`.
-- @setup
CREATE TABLE dtcu (a INT, b INT);

-- @expect-error: column "zz" of relation "dtcu" does not exist (42703)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtcu (zz, a, a) VALUES (1, 2, 3);
