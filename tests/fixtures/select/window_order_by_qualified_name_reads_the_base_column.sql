-- @setup
CREATE TABLE wz (a INT, b INT);
INSERT INTO wz VALUES (1, 5), (1, 7), (2, 5);

-- A **qualified** key never takes the output-name branch at all, whatever it is
-- nested in — so `wz.a` is the base column even though the select list carries an
-- output column called `a`. Same data, same shadowing, as the nested-name partner;
-- this one is the key-is-the-reference spelling of it.
-- @test
SELECT b AS a, row_number() OVER () AS rn FROM wz ORDER BY wz.a;
