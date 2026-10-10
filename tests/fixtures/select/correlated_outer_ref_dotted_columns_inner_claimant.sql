-- The innermost-wins polarity of
-- `select/correlated_outer_ref_dotted_columns_not_ambiguous.sql`: the inner relation
-- carries a real bare `c` beside the quoted `"a.c"`, so the reference binds *there* and
-- every outer row survives the `EXISTS`. The suffix match saw two claimants and raised
-- `42702` for this spelling too, which is what shows the defect was the match and not
-- the shadowing rule.
-- @setup
CREATE TABLE dcic_o (c INT);
CREATE TABLE dcic_i ("a.c" INT, c INT);
CREATE TABLE dcic_z (z INT);
INSERT INTO dcic_o VALUES (1), (2);
INSERT INTO dcic_i VALUES (10, 1);
INSERT INTO dcic_z VALUES (99);

-- @test
SELECT c FROM dcic_o WHERE EXISTS (SELECT 1 FROM dcic_i, dcic_z WHERE c = 1) ORDER BY c;
