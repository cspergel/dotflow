// DotFlow mark — three dots flowing along a rising curve ("dots" + "flow"). Uses currentColor so it matches
// the surrounding text (e.g. the sidebar). (File/export name kept for import compatibility.)
const HandyHand = ({
  width,
  height,
  className,
}: {
  width?: number | string;
  height?: number | string;
  className?: string;
}) => (
  <svg
    width={width || 24}
    height={height || 24}
    viewBox="0 0 24 24"
    fill="none"
    className={className}
    xmlns="http://www.w3.org/2000/svg"
  >
    {/* the flow */}
    <path
      d="M3 17c4 0 5.5-4.5 9-6.5S18 6 21 5.5"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      opacity="0.55"
    />
    {/* the dots, largest -> smallest as they flow off */}
    <circle cx="3.5" cy="17" r="2.4" fill="currentColor" />
    <circle cx="12" cy="10.5" r="1.9" fill="currentColor" />
    <circle cx="20.5" cy="5.5" r="1.5" fill="currentColor" />
  </svg>
);

export default HandyHand;
