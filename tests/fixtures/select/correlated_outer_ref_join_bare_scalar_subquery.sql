-- The projection-position spelling of `select/correlated_outer_ref_join_bare_column.sql`:
-- a **bare** outer reference into a join scope from a scalar subquery. The outer frame
-- addressed its columns by synthesized `"relation.column"` strings, so nothing was spelled
-- `k` and the statement was `42703` where 18.6 binds the one relation that claims it. The
-- qualified spelling answered on both engines throughout, which is what made it loud
-- rather than wrong.
-- @setup
CREATE TABLE jbs_t (id INT, k TEXT);
CREATE TABLE jbs_s (id INT, sk TEXT);
CREATE TABLE jbs_y (yk TEXT);
INSERT INTO jbs_t VALUES (1, 'hit'), (2, 'miss');
INSERT INTO jbs_s VALUES (1, 'sa'), (2, 'sb');
INSERT INTO jbs_y VALUES ('hit');

-- @test
SELECT (SELECT count(*) FROM jbs_y WHERE jbs_y.yk = k) AS n, jbs_t.id
FROM jbs_t JOIN jbs_s s ON s.id = jbs_t.id ORDER BY jbs_t.id;
