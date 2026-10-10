-- The test is the row's identity, not its key. The first proposal (key 1) moves the row it
-- updated onto key 11; the second proposal names key 11 — a key no earlier proposal named —
-- and meets that same row, which 18.6 refuses `21000`. A refusal keyed on a key proposed
-- twice is green here and red on `on_conflict_do_update_moved_row_frees_its_key.sql`.
-- @profile-lock
-- @setup
CREATE TABLE ocmn (k INT PRIMARY KEY, v TEXT);
INSERT INTO ocmn VALUES (1, 'a');

-- @test
-- @expect-error: ON CONFLICT DO UPDATE command cannot affect row a second time (21000)
INSERT INTO ocmn VALUES (1, 'x'), (11, 'y')
ON CONFLICT (k) DO UPDATE SET k = ocmn.k + 10, v = excluded.v;
