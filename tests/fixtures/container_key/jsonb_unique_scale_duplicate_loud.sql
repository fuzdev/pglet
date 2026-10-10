-- `jsonb`'s equality folds a number's scale, and a unique index enforces that equality:
-- `1.0` after `1` is a duplicate.
-- @profile-lock
-- @setup
CREATE TABLE cku_j (id INT, j JSONB UNIQUE);
INSERT INTO cku_j (j, id) VALUES ('1', 1), ('"1"', 2), ('[1]', 3), (NULL, 4), (NULL, 5);
-- @test
-- @expect-error: a scale-folded jsonb duplicate (23505)
-- @expect-message
INSERT INTO cku_j (j, id) VALUES ('1.0', 6);
