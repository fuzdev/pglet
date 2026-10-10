-- The accept polarity of the `RETURNING` binding check, over **zero** rows — the row
-- count the check newly reaches. Every legal spelling must still resolve: the target's
-- own name as a qualifier, a bare column, and an expression over each. An
-- over-strict binding rule (one that rejected the target's own name, or a bare name, or
-- a `*`) reddens here rather than passing unnoticed, and the declared-type legs compare
-- all three OIDs live against Postgres.
-- @setup
CREATE TABLE rzd (id INT, s SMALLINT, b BIGINT);
INSERT INTO rzd VALUES (1, 11, 111);

-- @test
DELETE FROM rzd WHERE id = 999 RETURNING rzd.b + 0 AS qual_expr, id, s + 0 AS bare_expr;
