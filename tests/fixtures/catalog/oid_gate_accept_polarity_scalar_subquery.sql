-- The accept side of the declaration-only operand gate at its **scalar-subquery** leaf.
-- The gate reads an operand's declared type through the complete walk, whose subquery
-- leaf is the plan's first output column; a leaf that answered the wrong type — or a
-- gate that refused whatever it could not resolve — would refuse every line here, all of
-- which 18.6 answers.
--
-- The nested chain is the second subject: an enclosing arithmetic node reads the type
-- its operand derived rather than re-deriving it, so a chain has to agree with the
-- single step at every depth.
-- @setup
CREATE TABLE oid_sub (qty INT);
INSERT INTO oid_sub VALUES (7);

-- @test
SELECT (SELECT qty FROM oid_sub) + 1 AS shifted,
       ((SELECT qty FROM oid_sub) + 1) + 1 AS shifted_twice,
       (SELECT max(qty) FROM oid_sub) * 2 AS doubled,
       -(SELECT qty FROM oid_sub) AS negated,
       abs((SELECT qty FROM oid_sub)) AS measured,
       ((SELECT qty FROM oid_sub))::text || 'x' AS concatenated
