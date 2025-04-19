" Install vim-plug if not found
if empty(glob('~/.vim/autoload/plug.vim'))
  silent! curl -fLo ~/.vim/autoload/plug.vim --create-dirs
    \ https://raw.githubusercontent.com/junegunn/vim-plug/master/plug.vim
endif

" Run PlugInstall if there are missing plugins
autocmd VimEnter * if len(filter(values(g:plugs), '!isdirectory(v:val.dir)'))
  \ | PlugInstall --sync | source $MYVIMRC
\ | endif

call plug#begin()

Plug 'catppuccin/vim', { 'as': 'catppuccin' }

call plug#end()

set termguicolors
silent! colorscheme catppuccin_mocha " catppuccin_frappe, catppuccin_latte, catppuccin_macchiato, catppuccin_mocha

