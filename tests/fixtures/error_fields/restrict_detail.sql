-- 23001's DETAIL is the delete-side sentence **without** the word "still" — the one
-- byte-level difference between the two blocked-parent shapes.
-- @setup
CREATE TABLE p_r (id INT PRIMARY KEY);
CREATE TABLE c_r (a INT REFERENCES p_r(id) ON DELETE RESTRICT);
INSERT INTO p_r VALUES (1);
INSERT INTO c_r VALUES (1);

-- @test
-- @expect-error: RESTRICT blocks the parent delete
-- @profile-lock
-- @expect-message
DELETE FROM p_r WHERE id = 1;
