-- A `numeric[]` key folds each element's scale, as `numeric`'s equality does: `{1}`
-- duplicates `{1.0}`.
-- @profile-lock
-- @setup
CREATE TABLE cku_n (id INT, a NUMERIC[] UNIQUE);
INSERT INTO cku_n (a, id) VALUES ('{1.0}', 1), ('{NaN}', 2), ('{0.00}', 3);
-- @test
-- @expect-error: a scale-folded numeric[] duplicate (23505)
-- @expect-message
INSERT INTO cku_n (a, id) VALUES ('{1}', 4);
