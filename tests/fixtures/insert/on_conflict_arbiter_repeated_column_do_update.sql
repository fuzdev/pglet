-- The repeat collapses for a `DO UPDATE` action too — the inference happens before the
-- action is reached, so the action runs and the row is updated.
-- @setup
CREATE TABLE oaru (a INT PRIMARY KEY, b INT);
INSERT INTO oaru VALUES (1, 1);
INSERT INTO oaru VALUES (1, 9) ON CONFLICT (a, a) DO UPDATE SET b = 42;

-- @profile-lock
-- @test
SELECT a, b FROM oaru ORDER BY a;
