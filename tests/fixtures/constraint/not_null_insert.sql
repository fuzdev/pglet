-- @setup
CREATE TABLE nn (id INT, name TEXT NOT NULL);

-- @test
-- @expect-error: null value violates not-null constraint
INSERT INTO nn (id, name) VALUES (1, NULL);
