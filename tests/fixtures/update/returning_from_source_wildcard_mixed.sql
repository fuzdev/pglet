-- A bare `RETURNING *` **mixed with other items** over an `UPDATE … FROM`.
--
-- The wildcard expands in place and the following items resolve after it, in source
-- order — so the widened `*` and an ordinary source reference coexist in one list, and
-- the labelled `s.k + 1` proves the expansion did not consume the rest of the clause.
-- The same statement with the wildcard alone is `returning_from_bare_wildcard.sql`.
-- @setup
CREATE TABLE rwm_t (t_id INT, t_v TEXT);
CREATE TABLE rwm_s (s_id INT, s_v TEXT, s_k INT);
INSERT INTO rwm_t VALUES (1, 'TARGET');
INSERT INTO rwm_s VALUES (1, 'SOURCE', 7);

-- @test
UPDATE rwm_t SET t_id = 1 FROM rwm_s s WHERE s.s_id = rwm_t.t_id RETURNING *, s.s_k + 1 AS kp;
