-- An `ON CONFLICT DO UPDATE` that moves a referenced key runs the same `ON UPDATE CASCADE`
-- as an `UPDATE`, through the `varchar(3)` child column's conversion: `'abcdef'` is refused
-- `22001`. Unconverted, the child held `'abcdef'`.
-- @setup
CREATE TABLE p (k varchar(10) PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk varchar(3) REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES ('a');
INSERT INTO c VALUES (1, 'a');
-- @test
-- @profile-lock
-- @expect-error: value too long for type character varying(3)
INSERT INTO p VALUES ('a') ON CONFLICT (k) DO UPDATE SET k = 'abcdef';
