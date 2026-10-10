-- Control: a scalar subquery projecting an enclosing column as the value an indexed
-- column is compared with, over an inner table carrying a same-named `text` column.
-- @setup
CREATE TABLE ctp_pi (a INT PRIMARY KEY, w TEXT);
CREATE TABLE ctp_te (m TEXT, k INT);
CREATE TABLE ctp_tt (m TEXT, k TEXT);
INSERT INTO ctp_pi VALUES (1, 'one'), (2, 'two');
INSERT INTO ctp_te VALUES ('a', 1), ('b', 2);
INSERT INTO ctp_tt VALUES ('x', 'y');
-- @test
SELECT k, (SELECT count(*) FROM ctp_pi WHERE ctp_pi.a = (SELECT o.k FROM ctp_tt)) AS c
  FROM ctp_te o ORDER BY k;
