-- @setup
-- Postgres allows selecting any column when the table's full primary key is in
-- the GROUP BY (functional dependency): `GROUP BY pk` legitimizes `name`/`other`
-- even though they aren't grouped. pglet's plan-time grouping check models this.
CREATE TABLE grp_fd (pk INT PRIMARY KEY, name TEXT, other TEXT);
INSERT INTO grp_fd VALUES (1, 'a', 'x'), (2, 'a', 'y');

-- @test
SELECT pk, name, other, COUNT(*) AS n FROM grp_fd GROUP BY pk ORDER BY pk;
