-- The rule is **more than one assignment**, not *conflicting* assignments: two
-- assignments of the same value to the same column are refused exactly as two different
-- values are. `process_matched_tle` is reached by the second entry's existence, and what
-- it then asks about the expressions is only whether they can be *merged* into one
-- field-store — which a whole-column assignment never can.
--
-- Read as "conflicting", the gate would have been written as a value comparison and this
-- file is the only one in the family that would have caught it.
-- @setup
CREATE TABLE dsti (a INT, b INT);
INSERT INTO dsti VALUES (0, 0);

-- @expect-error: multiple assignments to same column "a" (42601)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dsti SET a = 1, a = 1;
