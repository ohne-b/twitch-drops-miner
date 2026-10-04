import { Link, useLocation, useNavigate, type LinkProps } from 'react-router';

// Carry the originating view through the detail route, including its nested list scroll.
export function CampaignLink({ id, to, ...props }: LinkProps & { id: string }) {
  const location = useLocation();
  const navigate = useNavigate();
  return (
    <Link
      {...props}
      id={id}
      to={to}
      onClick={(event) => {
        if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
        event.preventDefault();
        navigate(to, {
          state: {
            campaignDetail: true,
            campaignReturn: {
              path: location.pathname + location.search + location.hash,
              focus: id,
              top: window.scrollY,
              lists: Array.from(document.querySelectorAll<HTMLElement>('[data-restore-scroll]'))
                .map((element) => [element.id, element.scrollTop]),
            },
          },
        });
      }}
    />
  );
}
