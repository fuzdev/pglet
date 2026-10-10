-- The correlated outer reference inside a **scalar subquery in the select list**. This
-- position is the one that cannot be fixed by reordering the parser: a projection's
-- tokens precede the FROM clause's, and the token stream has no rewind, so the outer
-- relations are unknown when the subquery is lowered. The enclosing level re-resolves
-- the sub-plan after `lower_select` builds its scope, which is *after* it has walked
-- the FROM clause — the same FROM-before-projection order Postgres gets from analyzing
-- a whole parsed tree.
-- @setup
CREATE TABLE cp_o (okey INT, ouid UUID);
CREATE TABLE cp_i (ikey INT, itag TEXT);
INSERT INTO cp_o VALUES (1, '11111111-1111-1111-1111-111111111111'), (2, '22222222-2222-2222-2222-222222222222');
INSERT INTO cp_i VALUES (1, 'a'), (2, 'b');
-- @test
SELECT okey, (SELECT count(*) FROM cp_i WHERE cp_o.ouid = '11111111-1111-1111-1111-111111111111') AS hits FROM cp_o ORDER BY okey;
