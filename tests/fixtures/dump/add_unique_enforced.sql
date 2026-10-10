-- ALTER TABLE ... ADD CONSTRAINT ... UNIQUE must build a real enforced unique
-- index, so a later duplicate is rejected with 23505 — matching Postgres.
-- @profile-lock
-- @setup
CREATE TABLE duq (id int, email text);
INSERT INTO duq VALUES (1, 'a@x'), (2, 'b@x');
ALTER TABLE ONLY duq
    ADD CONSTRAINT duq_email_key UNIQUE (email);
-- @test
INSERT INTO duq VALUES (3, 'a@x');
-- @expect-error: duplicate key violates the UNIQUE added by ADD CONSTRAINT
