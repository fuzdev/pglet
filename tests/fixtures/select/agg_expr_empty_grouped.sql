-- Under a `GROUP BY` an empty input forms no group: no row.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT abs(max(a)) AS m, concat('n=', count(*)) AS c FROM u WHERE false GROUP BY a;
