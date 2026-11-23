-- A composite foreign key and one declared under its own name drop by that name, quoted
-- or folded as any identifier is, with `ONLY` and a `CASCADE` / `RESTRICT` tail, and the
-- referencing columns keep their values.
-- @profile-lock
-- @setup
CREATE TABLE dccd_p (a INT, b INT, PRIMARY KEY (a, b));
CREATE TABLE dccd_c (
  x INT, y INT, z INT,
  FOREIGN KEY (x, y) REFERENCES dccd_p(a, b),
  CONSTRAINT "Dccd_Mixed" FOREIGN KEY (y, z) REFERENCES dccd_p(a, b),
  CONSTRAINT dccd_plain FOREIGN KEY (z, x) REFERENCES dccd_p(a, b)
);
INSERT INTO dccd_p VALUES (1, 1);
INSERT INTO dccd_c VALUES (1, 1, 1);

-- @test
ALTER TABLE ONLY dccd_c DROP CONSTRAINT dccd_c_x_y_fkey CASCADE, DROP CONSTRAINT "Dccd_Mixed" RESTRICT, DROP CONSTRAINT DCCD_PLAIN;
INSERT INTO dccd_c VALUES (7, 8, 9);
SELECT x, y, z, (SELECT count(*) FROM pg_constraint WHERE contype = 'f') AS fks
FROM dccd_c ORDER BY x;
