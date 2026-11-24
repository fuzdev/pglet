-- `FOR KEY SHARE OF … NOWAIT` over a join: the policy rides the whole statement.
-- @setup
CREATE TABLE lnowait (id INT PRIMARY KEY, code TEXT);
INSERT INTO lnowait VALUES (1, 'a'), (2, 'b');
CREATE TABLE lnowait_ref (rid INT, id INT);
INSERT INTO lnowait_ref VALUES (10, 1), (20, 2);
-- @test
SELECT r.rid, n.code FROM lnowait_ref r JOIN lnowait n ON n.id = r.id ORDER BY r.rid FOR KEY SHARE OF n NOWAIT;
