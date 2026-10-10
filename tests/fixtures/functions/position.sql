-- position: the POSITION(sub IN str) special form (arg-swapped to strpos) and the
-- strpos(str, sub) function form. 1-based character position, 0 when absent, 1 for
-- an empty needle.
-- @setup
CREATE TABLE p (id INT, t TEXT);
INSERT INTO p VALUES (1, 'Thomas'), (2, 'abc'), (3, NULL);
-- @test
SELECT id,
  position('om' IN t) AS pos,
  position('z' IN t) AS pos_absent,
  position('' IN t) AS pos_empty,
  strpos(t, 'a') AS sp
FROM p ORDER BY id;
