import { useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import type { GameMetadata } from '../../shared/lib/types';
import { mdiPlus } from '@mdi/js';
import { useT } from '../../shared/lib/i18n';
import { Art, IconButton, Search } from '../../shared/ui';

type GameOption = { name: string; image?: string | null; metadata?: GameMetadata };

export default function GameSearch({
  value,
  onChange,
  options,
  onSelect,
  onResolve,
  disabled,
  loading,
  ready,
  children,
}: {
  value: string;
  onChange: (value: string) => void;
  options: GameOption[];
  onSelect: (name: string, metadata?: GameMetadata) => void;
  onResolve: () => void;
  disabled: boolean;
  loading: boolean;
  ready: boolean;
  children: ReactNode;
}) {
  const t = useT();
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const picker = useRef<HTMLDivElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const [nativePopover] = useState(() => typeof HTMLElement.prototype.showPopover === 'function');
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState<{ query: string; name: string } | null>(null);
  const visible = open && !!value.trim() && ready && !disabled;
  const activeIndex =
    active?.query === value ? options.findIndex((game) => game.name === active.name) : -1;

  function position() {
    const rect = input.current!.getBoundingClientRect();
    const viewport = window.visualViewport;
    const top = (viewport?.offsetTop ?? 0) + 8;
    const bottom = (viewport?.offsetTop ?? 0) + (viewport?.height ?? innerHeight) - 8;
    if (rect.bottom <= top || rect.top >= bottom) {
      setOpen(false);
      setActive(null);
      return;
    }
    const below = Math.max(0, bottom - rect.bottom - 4);
    const above = Math.max(0, rect.top - top - 4);
    const upward = below < 160 && above > below;
    Object.assign(popup.current!.style, {
      left: `${rect.left}px`,
      width: `${rect.width}px`,
      maxHeight: `${Math.min(320, upward ? above : below)}px`,
      top: `${upward ? rect.top - 4 : rect.bottom + 4}px`,
      transform: upward ? 'translateY(-100%)' : '',
    });
  }

  useLayoutEffect(() => {
    // Save-status banners can move the field without resizing it.
    if (visible) position();
  });

  useLayoutEffect(() => {
    const element = popup.current!;
    const anchor = input.current!;
    if (!visible) {
      element.hidePopover?.();
      if (disabled) {
        setOpen(false);
        setActive(null);
      }
      return;
    }
    element.showPopover?.({ source: anchor });
    function dismiss(event: PointerEvent) {
      if (!picker.current?.contains(event.target as Node)) {
        setOpen(false);
        setActive(null);
      }
    }
    if (!nativePopover) document.addEventListener('pointerdown', dismiss);
    const observer = new ResizeObserver(position);
    observer.observe(anchor);
    window.addEventListener('resize', position);
    document.addEventListener('scroll', position, true);
    window.visualViewport?.addEventListener('resize', position);
    window.visualViewport?.addEventListener('scroll', position);
    return () => {
      observer.disconnect();
      document.removeEventListener('pointerdown', dismiss);
      window.removeEventListener('resize', position);
      document.removeEventListener('scroll', position, true);
      window.visualViewport?.removeEventListener('resize', position);
      window.visualViewport?.removeEventListener('scroll', position);
    };
  }, [visible, disabled, nativePopover]);

  useLayoutEffect(() => {
    if (visible && activeIndex >= 0)
      document.getElementById(`${id}-${activeIndex}`)?.scrollIntoView({ block: 'nearest' });
  }, [visible, activeIndex, id]);

  function select(game: GameOption) {
    setOpen(false);
    setActive(null);
    onSelect(game.name, game.metadata);
    input.current?.focus({ preventScroll: true });
  }

  return (
    <div
      ref={picker}
      className="flex min-w-0 flex-1 gap-2 max-md:basis-full"
      onKeyDownCapture={(event) => {
        if (event.key !== 'Escape' || event.nativeEvent.isComposing) return;
        event.preventDefault();
        event.stopPropagation();
        if (popup.current?.contains(document.activeElement))
          input.current?.focus({ preventScroll: true });
        setOpen(false);
        setActive(null);
      }}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          setOpen(false);
          setActive(null);
        }
      }}
    >
      <fieldset disabled={disabled} aria-busy={loading} className="min-w-0 flex-1">
        <Search
          inputRef={input}
          value={value}
          label={t('gui.settings.search_games')}
          onChange={(query) => {
            setActive(null);
            setOpen(true);
            onChange(query);
          }}
          inputProps={{
            role: 'combobox',
            autoComplete: 'off',
            'aria-autocomplete': 'list',
            'aria-expanded': visible,
            'aria-controls': visible ? `${id}-options` : undefined,
            'aria-activedescendant':
              visible && activeIndex >= 0 ? `${id}-${activeIndex}` : undefined,
            onFocus: () => setOpen(true),
            onClick: () => setOpen(true),
            onKeyDown: (event) => {
              if (event.nativeEvent.isComposing) return;
              if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                event.preventDefault();
                setOpen(true);
                const index =
                  event.key === 'ArrowDown'
                    ? Math.min(options.length - 1, activeIndex + 1)
                    : activeIndex < 0
                      ? options.length - 1
                      : Math.max(0, activeIndex - 1);
                const game = options[index];
                if (game) setActive({ query: value, name: game.name });
              } else if (event.key === 'Enter') {
                event.preventDefault();
                if (visible && options[activeIndex]) select(options[activeIndex]);
                else onResolve();
              }
            },
          }}
        />
      </fieldset>
      <IconButton
        path={mdiPlus}
        label={t('gui.settings.add_game')}
        disabled={disabled || !value.trim() || loading}
        onClick={() => {
          setOpen(true);
          onResolve();
          input.current?.focus({ preventScroll: true });
        }}
      />
      <div
        ref={popup}
        popover={nativePopover ? 'auto' : undefined}
        hidden={!nativePopover && !visible}
        data-fallback-open={!nativePopover && visible ? true : undefined}
        className="game-search-popover"
        onClick={() => input.current?.focus({ preventScroll: true })}
        onBeforeToggle={(event) => {
          if (event.newState === 'closed' && visible) {
            setOpen(false);
            setActive(null);
          }
        }}
      >
        {children}
        <div
          id={`${id}-options`}
          role="listbox"
          tabIndex={-1}
          aria-label={t('gui.settings.search_games')}
          className="scroll-list min-h-0 overflow-y-auto overscroll-contain"
        >
          {options.map((game, index) => (
            <div
              key={game.name}
              id={`${id}-${index}`}
              role="option"
              aria-selected={activeIndex === index}
              className={`flex min-h-11 cursor-pointer items-center gap-3 px-3 py-2 text-[13px] hover:bg-hover ${activeIndex === index ? 'bg-hover' : ''}`}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => select(game)}
            >
              <Art url={game.image} className="size-8" />
              <span>{game.name}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
