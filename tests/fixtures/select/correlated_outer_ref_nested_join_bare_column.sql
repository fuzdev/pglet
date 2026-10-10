-- `select/correlated_outer_ref_join_bare_column.sql` one join deeper. The nested join's
-- output columns arrive at the outer level already synthesized, so a derivation that
-- re-parses their names has to be right about a spelling it produced itself; the pairs
-- come from the plan and the depth costs nothing.
-- @setup
CREATE TABLE njb_a (id INT, w TEXT);
CREATE TABLE njb_b (id INT, y TEXT);
CREATE TABLE njb_c (id INT, q TEXT);
CREATE TABLE njb_z (k TEXT);
INSERT INTO njb_a VALUES (1, 'hit'), (2, 'miss');
INSERT INTO njb_b VALUES (1, 'yy'), (2, 'zz');
INSERT INTO njb_c VALUES (1, 'cc'), (2, 'dd');
INSERT INTO njb_z VALUES ('hit');

-- @test
SELECT njb_a.id FROM njb_a JOIN njb_b ON njb_b.id = njb_a.id JOIN njb_c ON njb_c.id = njb_a.id
WHERE EXISTS (SELECT 1 FROM njb_z WHERE njb_z.k = w);
