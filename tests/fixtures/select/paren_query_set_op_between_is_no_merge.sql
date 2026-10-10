-- With a set operation after the parenthesized query, the trailing `LIMIT` bounds the set
-- operation and does not meet the operand's own.
-- @setup
CREATE TABLE pqu_s (x INT);
INSERT INTO pqu_s VALUES (2), (1);

-- @test
(SELECT x FROM pqu_s ORDER BY x LIMIT 1) UNION ALL (SELECT x FROM pqu_s ORDER BY x DESC LIMIT 1) ORDER BY 1 LIMIT 1;
