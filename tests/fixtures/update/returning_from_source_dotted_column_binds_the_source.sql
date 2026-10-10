-- A `RETURNING` name a user **quoted into existence with a dot in it**, spelled so it
-- collides with the name the executor synthesizes for the *target's* column.
--
-- The executor addressed a multi-relation `RETURNING` namespace by synthesized
-- `relation.column` strings, and those are byte-identical to a quoted user column name.
-- The target here is `rdc_t(id, v)`, so its `v` synthesizes to `rdc_t.v`; the source
-- carries a real column named `"rdc_t.v"`. The resolver's exact tier ran first and won
-- outright, so the bare reference answered the **target's** `v` where 18.6 answers the
-- source's column — no error on either engine, on a statement both accept.
--
-- The values are chosen so a mis-binding is a *different string* (`TARGETV` against
-- `SRCDOTTED`) rather than the same one, and the qualified control rides in the same row:
-- `s."rdc_t.v"` was correct on both engines before and after, which is what identifies
-- the **unqualified** resolution as the cause rather than the column name.
--
-- Its opposite polarity — a target column whose name collides with the *source's*
-- synthesized name — is `returning_from_target_dotted_column_binds_the_target.sql`.
-- @setup
CREATE TABLE rdc_t (id INT, v TEXT);
CREATE TABLE rdc_s (id INT, "rdc_t.v" TEXT);
INSERT INTO rdc_t VALUES (1, 'TARGETV');
INSERT INTO rdc_s VALUES (1, 'SRCDOTTED');

-- @test
UPDATE rdc_t SET id = rdc_t.id FROM rdc_s s WHERE s.id = rdc_t.id
  RETURNING "rdc_t.v" AS bare, s."rdc_t.v" AS qualified, rdc_t.v AS target_v;
