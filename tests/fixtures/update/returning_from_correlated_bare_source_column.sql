-- The `UPDATE … FROM` spelling of the same bare outer reference: a `RETURNING` subquery
-- naming the **source**'s column with no qualifier. The clause's namespace is two
-- relations, and the frame that carried it addressed them by synthesized
-- `relation.column` strings, so `sk` matched nothing and this was `42703` where 18.6
-- answers. The seed is what makes it discriminate — `bsc_y` holds the source's value, so
-- a reference bound anywhere else counts `0` for both rows.
-- @setup
CREATE TABLE bsc_u (id INT, v TEXT);
CREATE TABLE bsc_s (id INT, sk TEXT);
CREATE TABLE bsc_y (yk TEXT);
INSERT INTO bsc_u VALUES (1, 'x'), (2, 'y');
INSERT INTO bsc_s VALUES (1, 'sa'), (2, 'sb');
INSERT INTO bsc_y VALUES ('sa');

-- @test
UPDATE bsc_u SET v = bsc_u.v FROM bsc_s s WHERE s.id = bsc_u.id
RETURNING (SELECT count(*) FROM bsc_y WHERE bsc_y.yk = sk) AS n, bsc_u.id;
