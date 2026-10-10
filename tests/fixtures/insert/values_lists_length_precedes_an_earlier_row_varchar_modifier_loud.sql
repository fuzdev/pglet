-- An unknown literal meets its column's input function in parse analysis with no length or
-- precision modifier (`interval` alone takes its fields and precision there); `varchar(3)`'s
-- length is applied later, in the planner. So the first
-- row's over-long `'abcdef'` loses to the second row's length: `42601`, not `22001`.
-- @setup
CREATE TABLE vlvm (a VARCHAR(3), b INT);

-- @test
-- @expect-error: VALUES lists must all be the same length (42601), ahead of the first row's 22001
-- @expect-message
INSERT INTO vlvm VALUES ('abcdef', 1), ('a');
