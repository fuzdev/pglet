-- The ambiguity polarity of `select/correlated_outer_ref_join_bare_column.sql`: both
-- sides of the join outer carry `w`, so 18.6 counts two claimants and raises `42702`.
--
-- Here the **plan-time** binding gate answers it, having the same count over the FROM
-- namespace, so it stays green under every mutation of the executor's frame — it is the
-- control that says the sibling's one-claimant resolution is not a first match, not a
-- guard on the frame. The frame's own count is pinned by
-- `executor::expr::tests::outer_frame_qualifier_tests::a_pair_frame_resolves_a_bare_name_one_relation_claims`,
-- which reaches the two-claimant arm the gate hides from SQL.
-- @setup
CREATE TABLE jba_a (id INT, w TEXT);
CREATE TABLE jba_b (id INT, w TEXT);
CREATE TABLE jba_z (k TEXT);
INSERT INTO jba_a VALUES (1, 'hit');
INSERT INTO jba_b VALUES (1, 'hit');
INSERT INTO jba_z VALUES ('hit');

-- @expect-error: column reference "w" is ambiguous (42702)
-- @test
SELECT jba_a.id FROM jba_a JOIN jba_b ON jba_b.id = jba_a.id
WHERE EXISTS (SELECT 1 FROM jba_z WHERE jba_z.k = w);
