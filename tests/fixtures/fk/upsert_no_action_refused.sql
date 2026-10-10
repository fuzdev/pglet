-- An `ON CONFLICT DO UPDATE` that moves a referenced key a child still holds is refused
-- by the child's `NO ACTION` foreign key, as the plain `UPDATE` is.
-- @profile-lock
-- @setup
CREATE TABLE upna_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upna_c (pid INT REFERENCES upna_p(id), x INT);
INSERT INTO upna_p VALUES (1, 1), (2, 2);
INSERT INTO upna_c VALUES (1, 10);

-- @test
-- @expect-error: the rewrite vacates key 1, which upna_c still references (23503)
INSERT INTO upna_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9;
