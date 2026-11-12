" Syntax highlighting for Lava (.lv files)

if exists("b:current_syntax")
  finish
endif

syn keyword lavaKeyword     func class enum trait impl Constructor pub import as for extern
syn keyword lavaStorage     let mut
syn keyword lavaConditional if else match
syn keyword lavaRepeat      while
" `value is pattern`, and `in` of `for item in items`, also `for (a, b) in pairs`.
" Elsewhere `in` is a name like any other
syn keyword lavaWordOp      is
" Starts at `in` itself, a match starting at `for` would lose to the keyword `for`
syn match   lavaWordOp      "\(\<for\s[^{]\{-}\s\)\@<=in\>"
syn keyword lavaStatement   return break continue
syn keyword lavaSelf        self
syn keyword lavaBoolean     true false
syn keyword lavaType        bool char byte String int8 int16 int32 int64 uint8 uint16 uint32 uint64 float32 float64 Unique Shared Buffer
syn keyword lavaBuiltin     print unique shared panic int_to_string float_to_string char_to_string
syn keyword lavaPrelude     Option Result Vector Map Some None Ok Err

syn match   lavaTypeName    "\<[A-Z][A-Za-z0-9_]*\>"
syn match   lavaFunction    "\<[a-z_][A-Za-z0-9_]*\ze\s*\(<[^>]*>\)\?\s*("
" Like `42`, `1_000_000`, `2.5` or `1e9`
syn match   lavaNumber      "\<\d\(_\?\d\)*\(\.\d\(_\?\d\)*\)\?\([eE][+-]\?\d\(_\?\d\)*\)\?\>"
syn match   lavaOperator    "=>\|->\|::\|==\|!=\|<=\|>=\|&&\|||\|[-+*/%=<>&!^?]"
" `a ?? b`, the value or a default, stands out from the other operators
syn match   lavaDefault     "??"
syn region  lavaString      start=+"+ skip=+\\.+ end=+"+ contains=lavaEscape
syn match   lavaEscape      contained +\\[nrt0\\"']+
syn match   lavaChar        +'\(\\[nrt0\\"']\|[^'\\]\)'+
syn match   lavaComment     "//.*$" contains=lavaTodo,@Spell
syn keyword lavaTodo        contained TODO FIXME XXX NOTE

hi def link lavaKeyword     Keyword
hi def link lavaStorage     StorageClass
hi def link lavaConditional Conditional
hi def link lavaRepeat      Repeat
hi def link lavaWordOp      Keyword
hi def link lavaStatement   Statement
hi def link lavaSelf        Special
hi def link lavaBoolean     Boolean
hi def link lavaType        Type
hi def link lavaTypeName    Type
hi def link lavaBuiltin     Function
hi def link lavaFunction    Function
hi def link lavaNumber      Number
hi def link lavaOperator    Operator
hi def link lavaDefault     Special
hi def link lavaString      String
hi def link lavaEscape      SpecialChar
hi def link lavaChar        Character
hi def link lavaPrelude     Type
hi def link lavaComment     Comment
hi def link lavaTodo        Todo

let b:current_syntax = "lava"
