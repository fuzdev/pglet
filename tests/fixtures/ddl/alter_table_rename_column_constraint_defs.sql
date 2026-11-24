-- The constraints over a renamed column follow it — the composite `UNIQUE`, the `CHECK`, and another table's foreign key referencing it — and keep their names.
-- @setup
CREATE TABLE cp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0), m int, UNIQUE (m, n));
CREATE INDEX cp_lower ON cp (lower(v)) WHERE n > 1;
CREATE TABLE cc (pid int REFERENCES cp (id), pm int, pn int, FOREIGN KEY (pm, pn) REFERENCES cp (m, n));
INSERT INTO cp (v, n, m) VALUES ('a', 5, 1), ('b', 7, 2);
-- @test
ALTER TABLE cp RENAME COLUMN n TO num;
SELECT conname, pg_get_constraintdef(oid) FROM pg_constraint WHERE conname IN ('cp_m_n_key', 'cp_n_check', 'cc_pm_pn_fkey') ORDER BY 1;
