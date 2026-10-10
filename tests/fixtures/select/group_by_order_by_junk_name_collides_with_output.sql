-- @setup
CREATE TABLE jc (k INT, pglet_junk_0 TEXT);
INSERT INTO jc VALUES (1, 'z'), (2, 'y'), (3, 'x');

-- A junk column is read back by **name**, and the resolver's exact tier takes the
-- first match — so a real output column wearing the generated name would answer in
-- the junk column's place. The generated name is therefore checked against the
-- node's actual output names rather than assumed unguessable. The text values run
-- opposite to `k` so the substitution is observable: taking the output column would
-- sort by 'x','y','z' instead.
-- @test
SELECT pglet_junk_0 FROM jc GROUP BY pglet_junk_0, k ORDER BY k DESC;
