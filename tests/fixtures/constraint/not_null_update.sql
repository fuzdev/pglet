-- @setup
CREATE TABLE nn (id INT, name TEXT NOT NULL);
INSERT INTO nn (id, name) VALUES (1, 'alice');

-- @test
-- @expect-error: UPDATE to NULL violates not-null constraint
UPDATE nn SET name = NULL WHERE id = 1;
