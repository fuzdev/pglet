-- The same auto-drop for a multi-column CHECK: dropping either operand takes the
-- whole constraint with it, so the surviving column is unconstrained afterwards
-- (a value the old predicate would have rejected is accepted).
-- @setup
CREATE TABLE dcmc (id INT PRIMARY KEY, a INT, b INT, CONSTRAINT dcmc_ab CHECK (a > b));
INSERT INTO dcmc VALUES (1, 9, 2);

-- @profile-lock
-- @test
ALTER TABLE dcmc DROP COLUMN b;
INSERT INTO dcmc VALUES (2, -5);
SELECT id, a FROM dcmc ORDER BY id;
