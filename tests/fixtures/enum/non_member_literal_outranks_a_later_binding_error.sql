-- The literal **conversion** at the same precedence position the operand gate holds in
-- `computed_text_operand_outranks_a_later_binding_error`: 18.6 resolves the target list in
-- source order, so item one's `enum_in` outranks item two's unbindable name. This shape
-- answered `42703` before the conversion ran at the fold at all.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
-- @expect-error: the conversion at item one outranks item two's unbound name
-- @expect-message
SELECT greatest(m, 'zzz'), nosuchcol FROM t;
