-- An INSERT's target column list is a **set**: a column named twice is
-- `42701 column "a" specified more than once`, from `checkInsertTargets`
-- (`parser/parse_target.c`), which tests each resolved name against the columns it has
-- already claimed. pglet accepted the list and wrote the **last** value, so this
-- statement left `a = 2` behind.
--
-- The family's other spellings are the siblings named `duplicate_target_column_*`; the
-- accept polarity is any list of distinct names, which every other insert fixture
-- carries.
-- @setup
CREATE TABLE dtc (a INT, b INT, c INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtc (a, a) VALUES (1, 2);
