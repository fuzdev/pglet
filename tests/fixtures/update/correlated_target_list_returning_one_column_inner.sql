-- A `RETURNING` subquery whose target list names the `UPDATE … FROM` source's column,
-- bare and qualified, over a one-column inner table that lacks it: 18.6 binds each to the
-- source row and answers. A binding check over the inner table alone refused `42703`.
-- @setup
CREATE TABLE ctr2_u (id INT, v TEXT);
CREATE TABLE ctr2_s (id INT, sk TEXT);
CREATE TABLE ctr2_y (yk TEXT);
INSERT INTO ctr2_u VALUES (1, 'x'), (2, 'y');
INSERT INTO ctr2_s VALUES (1, 'sa'), (2, 'sb');
INSERT INTO ctr2_y VALUES ('q');
-- @test
UPDATE ctr2_u SET v = ctr2_u.v FROM ctr2_s s WHERE s.id = ctr2_u.id
RETURNING ctr2_u.id, (SELECT sk FROM ctr2_y) AS bare, (SELECT s.sk FROM ctr2_y) AS qualified;
