-- Both directions in one statement: `triuo_owned` is owned by `a` and restarts even
-- though no default names it, while `triuo_used` is what `a` actually defaults from
-- and is left alone. The column-default reading answers exactly backwards.
-- @setup
CREATE TABLE triuo_t (a INT);
CREATE SEQUENCE triuo_owned START WITH 30;
CREATE SEQUENCE triuo_used START WITH 60;
ALTER SEQUENCE triuo_owned OWNED BY triuo_t.a;
ALTER TABLE triuo_t ALTER COLUMN a SET DEFAULT nextval('triuo_used');
INSERT INTO triuo_t (a) VALUES (DEFAULT);
SELECT nextval('triuo_owned');
TRUNCATE triuo_t RESTART IDENTITY;
-- @test
SELECT nextval('triuo_owned') AS owned_restarted, nextval('triuo_used') AS used_untouched;
