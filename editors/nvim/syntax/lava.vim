" Syntax highlighting for Lava (.lv files)

if exists("b:current_syntax")
  finish
endif

syn keyword lavaKeyword     func class enum trait impl Constructor pub import as for
syn keyword lavaStorage     let mut
syn keyword lavaConditional if else match
syn keyword lavaRepeat      while
syn keyword lavaStatement   return
syn keyword lavaSelf        self
syn keyword lavaBoolean     true false
syn keyword lavaType        bool String int8 int16 int32 int64 uint8 uint16 uint32 uint64 float32 float64 Unique Buffer
syn keyword lavaBuiltin     print unique panic sqrt

syn match   lavaTypeName    "\<[A-Z][A-Za-z0-9_]*\>"
syn match   lavaFunction    "\<[a-z_][A-Za-z0-9_]*\ze\s*\(<[^>]*>\)\?\s*("
syn match   lavaNumber      "\<\d\+\(\.\d\+\)\?\>"
syn match   lavaOperator    "=>\|->\|::\|==\|!=\|<=\|>=\|[-+*/=<>&]"
syn region  lavaString      start=+"+ end=+"+
syn match   lavaComment     "//.*$" contains=lavaTodo,@Spell
syn keyword lavaTodo        contained TODO FIXME XXX NOTE

hi def link lavaKeyword     Keyword
hi def link lavaStorage     StorageClass
hi def link lavaConditional Conditional
hi def link lavaRepeat      Repeat
hi def link lavaStatement   Statement
hi def link lavaSelf        Special
hi def link lavaBoolean     Boolean
hi def link lavaType        Type
hi def link lavaTypeName    Type
hi def link lavaBuiltin     Function
hi def link lavaFunction    Function
hi def link lavaNumber      Number
hi def link lavaOperator    Operator
hi def link lavaString      String
hi def link lavaComment     Comment
hi def link lavaTodo        Todo

let b:current_syntax = "lava"
