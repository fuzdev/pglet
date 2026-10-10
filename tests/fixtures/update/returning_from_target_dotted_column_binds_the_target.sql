-- The opposite polarity of
-- `returning_from_source_dotted_column_binds_the_source.sql`: the quoted dotted column
-- is the **target's**, and it collides with the name the executor synthesizes for the
-- *source's* column.
--
-- The target carries `"s.k"`; the source is aliased `s` and carries `k`, which
-- synthesized to `s.k`. So the bare reference answered the source's `k` where 18.6
-- answers the target's column. Both directions are pinned because the right answer is a
-- *different relation* in each: a rule that always prefers the target is correct here and
-- wrong in the source-side twin, and a rule that always prefers the source is correct
-- there and wrong here. Neither file alone can tell such a rule from the claimant count.
--
-- This polarity is the stronger of the two, and the reason is the **type**: the wrong
-- column is `int4` where the right one is `text`, so a build that mis-binds also
-- declares the wrong type — measured on the simple-query leg (`int4` where 18.6 and
-- pglet's own `Describe` both said `text`), which put four raw `int4` bytes under a
-- `text` OID for a binary client. The harness compares the declared type OID on both
-- legs, so that half reddens here even where the values would not.
-- @setup
CREATE TABLE rdt_t (id INT, "s.k" TEXT);
CREATE TABLE rdt_s (id INT, v TEXT, k INT);
INSERT INTO rdt_t VALUES (1, 'TARGETDOTTED');
INSERT INTO rdt_s VALUES (1, 'sv', 7);

-- @test
UPDATE rdt_t SET id = rdt_t.id FROM rdt_s s WHERE s.id = rdt_t.id
  RETURNING "s.k" AS bare, rdt_t."s.k" AS qualified, s.k AS source_k;
